use image::{imageops::FilterType, DynamicImage, ImageFormat};
use kube::client::Client;
use s3::bucket::Bucket;
use scopeguard::defer;
use std::{env, ffi::OsStr, path::Path, process::Stdio};
use tokio::process::Command;
use tokio::{fs, io::BufReader};
use ytdl_common::{get_thumbnail_output, get_video_output, Error, Output};
use ytdl_types::{Executor, ThumbnailStorageSpec};

/// Path for the metadata info json file. youtube-dl can only
/// load this from a file, and it's convenient to write it out
/// for debugging purposes (e.g. `cat /info.json`).
const INFO_JSON_PATH: &str = "/info.json";

pub async fn download(client: Client, command: &str, dl_video: bool, dl_thumbnail: bool) {
    // Parse the resource from the environment.
    let instance: Executor =
        get_resource().expect("failed to get Executor resource from environment");

    // Write the video metadata to a file so youtube-dl
    // won't query the video service again.
    fs::write(INFO_JSON_PATH, &instance.spec.metadata)
        .await
        .expect("failed to write video info json to file");

    // Parse the video metadata json from the spec.
    let metadata: serde_json::Value = instance
        .spec
        .metadata
        .parse()
        .expect("failed to parse video info json");

    // Get the extra args from the spec.
    let extra: Option<&str> = instance.spec.extra.as_deref();

    // Determine what we need to do, download-wise, and
    // get the output objects at the same time.
    let outputs = get_outputs(client, &metadata, &instance, dl_video, dl_thumbnail)
        .await
        .expect("failed to get outputs");

    // Wait for the VPN to connect before starting the download.
    println!("Environment parsed, waiting for VPN to connect");
    crate::ready::wait_for_vpn()
        .await
        .expect("vpn failed to connect");

    // Start the download(s).
    match outputs {
        // Download both video and thumbnail concurrently.
        (Some(video_output), Some(thumbnail_output)) => {
            let thumbnail_opts = get_thumbnail_options(&instance, &thumbnail_output.1)
                .expect("thumbnail output options");
            println!("Downloading video and thumbnail");
            let result = tokio::join!(
                download_video(&metadata, video_output.0, video_output.1, &command, extra),
                download_thumbnail(
                    &metadata,
                    thumbnail_opts,
                    thumbnail_output.0,
                    thumbnail_output.1
                ),
            );
            result.0.expect("failed to download video");
            result.1.expect("failed to download thumbnail");
        }
        // Download the video only.
        (Some(video_output), None) => {
            println!("Downloading video");
            download_video(&metadata, video_output.0, video_output.1, &command, extra)
                .await
                .expect("failed to download video");
        }
        // Download the thumbnail only.
        (None, Some(thumbnail_output)) => {
            let thumbnail_opts = get_thumbnail_options(&instance, &thumbnail_output.1)
                .expect("thumbnail output options");
            println!("Downloading thumbnail");
            download_thumbnail(
                &metadata,
                thumbnail_opts,
                thumbnail_output.0,
                thumbnail_output.1,
            )
            .await
            .expect("failed to download thumbnail");
        }
        (None, None) => {
            // The operator should never create an executor pod
            // without specifying at least one of the options.
            panic!("no download options specified");
        }
    }
}

/// A struct containing the processing options when downloading
/// thumbnails. To prevent a bucket from receiving thumbnails of
/// mixed formats, the user must specify and output format for
/// all thumbnails. If the user does not specify a format, the
/// format can be inferred from the output key. If the format
/// cannot be inferred at all, the download will fail.
/// If only one of the resize dimensions is set, the image
/// will be resized proportionally, keeping the aspect ratio.
struct ThumbnailOptions {
    /// Output format for the thumbnail. Conversion is enforced
    /// to normalize the format across all thumbnails.
    format: ImageFormat,

    /// Sampling filter to use when resizing.
    filter: FilterType,

    /// Maximum width (pixels) of the thumbnail image.
    width: Option<u32>,

    /// Maximum height (pixels) of the thumbnail image.
    height: Option<u32>,
}

/// Returns a struct containing download and processing options
/// for the thumbnail. The options are determined by the spec
/// and the output key is used to infer output format if it's
/// not specified explicitly in the spec.
fn get_thumbnail_options(instance: &Executor, key: &str) -> Result<ThumbnailOptions, Error> {
    // All of the thumbnail output options are specified in a single
    // section of the spec that addresses thumbnail storage.
    let thumbnail: &ThumbnailStorageSpec = instance.spec.output.thumbnail.as_ref().unwrap();
    // Determine the sampling filter to use when resizing.
    let filter: FilterType = match thumbnail.filter {
        // User can override the filter in the spec.
        Some(ref filter) => parse_filter_type(filter).ok_or_else(|| {
            Error::UserInputError(format!("unsupported image filter: {}", filter))
        })?,
        // Default filter is the highest quality.
        None => FilterType::Lanczos3,
    };
    // Determine the output image format, which may be
    // different from the downloaded thumbnail and will
    // necessitate conversion.
    let format: ImageFormat = match thumbnail.format {
        // Prefer the overridden format in the spec.
        Some(ref format) => ImageFormat::from_extension(format).ok_or_else(|| {
            Error::UserInputError(format!("unsupported thumbnail format: {}", format))
        })?,
        // Default to the format inferred from the output key.
        None => match get_format_from_filename(key) {
            // Output S3 key was a valid image extension.
            Some(format) => format,
            // Image format cannot be inferred from spec.
            None => return Err(Error::UserInputError(
                "thumbnail output format not specified and could not be inferred from output key"
                    .to_owned(),
            )),
        },
    };
    Ok(ThumbnailOptions {
        format,
        filter,
        width: thumbnail.width,
        height: thumbnail.height,
    })
}

/// Parses the Executor resource from the environment.
fn get_resource() -> Result<Executor, Error> {
    Ok(serde_json::from_str(&env::var("RESOURCE")?)?)
}

/// Error code for missing video output spec. The operator
/// should never ask an Executor pod to download a video
/// without providing an output spec. This is considered
/// an unreachable error.
const NO_VIDEO_OUTPUT: &str = "video output requested but no output spec provided";

/// Error code for missing thumbnail output spec. The operator
/// should never ask an Executor pod to download a thumbnail
/// without providing an output spec. This is considered
/// an unreachable error.
const NO_THUMBNAIL_OUTPUT: &str = "thumbnail output requested but no output spec provided";

/// Returns the output objects for the executor.
async fn get_outputs(
    client: Client,
    metadata: &serde_json::Value,
    instance: &Executor,
    download_video: bool,
    download_thumbnail: bool,
) -> Result<(Option<Output>, Option<Output>), Error> {
    match (download_video, download_thumbnail) {
        // The operator is asking this executor download both
        // the video and thumbnail. We can do this concurrently.
        (true, true) => {
            let result = tokio::join!(
                get_video_output(client.clone(), &metadata, &instance),
                get_thumbnail_output(client.clone(), &metadata, &instance),
            );
            let video_output = result.0?.expect(NO_VIDEO_OUTPUT);
            let thumbnail_output = result.1?.expect(NO_THUMBNAIL_OUTPUT);
            Ok((Some(video_output), Some(thumbnail_output)))
        }
        // Operator is asking this executor to download just the video.
        (true, false) => {
            let video_output = get_video_output(client, metadata, instance)
                .await?
                .expect(NO_VIDEO_OUTPUT);
            Ok((Some(video_output), None))
        }
        // Operator is asking this executor to download just the thumbnail.
        (false, true) => {
            let thumbnail_output = get_thumbnail_output(client, metadata, instance)
                .await?
                .expect(NO_THUMBNAIL_OUTPUT);
            Ok((None, Some(thumbnail_output)))
        }
        // Operator is asking this executor to download nothing.
        // This is an unreachable branch because the operator
        // should never create an executor pod without specifying
        // at least one of the download options.
        (false, false) => {
            panic!("no download options specified");
        }
    }
}

/// Builds the AV download command for youtube-dl.
/// Other commands (e.g. yt-dlp) are injected here.
fn build_cmd(command: &str, extra: Option<&str>) -> String {
    let mut cmd = format!("{} --load-info-json {} -o -", command, INFO_JSON_PATH,);
    if let Some(extra) = extra {
        cmd = format!("{} {}", cmd, extra);
    }
    cmd
}

/// Downloads the video and uploads it to the specified output.
async fn download_video(
    metadata: &serde_json::Value,
    bucket: Bucket,
    key: String,
    command: &str,
    extra: Option<&str>,
) -> Result<(), Error> {
    // We pass the webpage_url value as the query to youtub-dl.
    let webpage_url: &str = metadata
        .get("webpage_url")
        .ok_or_else(|| Error::UserInputError("metadata is missing webpage_url".to_owned()))?
        .as_str()
        .ok_or_else(|| Error::UserInputError("metadata webpage_url is not a string".to_owned()))?;
    println!(
        "Downloading video {} -> s3://{}/{}",
        webpage_url, &bucket.name, &key
    );
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&build_cmd(command, extra))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::UnknownError("failed to get child process stdout".to_owned()))?;
    let mut reader = BufReader::new(stdout);
    let status_code = bucket.put_object_stream(&mut reader, &key).await?;
    if status_code != 200 {
        return Err(Error::S3UploadError { status_code });
    }
    let status = child.wait().await?;
    if status.success() {
        // Upload completed and youtube-dl exited successfully.
        println!("Video download completed successfully");
        return Ok(());
    }
    let exit_code = status
        .code()
        .expect("youtube-dl failed with no exit status");
    Err(Error::YoutubeDlError { exit_code })
}

/// Returns the default thumbnail url from the video infojson.
fn get_thumbnail_url(metadata: &serde_json::Value) -> Result<String, Error> {
    Ok(metadata
        .get("thumbnail")
        .ok_or_else(|| Error::UserInputError("metadata is missing thumbnail".to_owned()))?
        .as_str()
        .ok_or_else(|| Error::UserInputError("metadata thumbnail is not a string".to_owned()))?
        .to_owned())
}

/// Converts the HTTP response Content-Type header
/// to the corresponding image format enum value.
fn mimetype_to_format(mimetype: &str) -> Result<ImageFormat, Error> {
    Ok(match mimetype {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        "image/tiff" => ImageFormat::Tiff,
        "image/bmp" => ImageFormat::Bmp,
        "image/x-icon" => ImageFormat::Ico,
        _ => {
            return Err(Error::UserInputError(format!(
                "unsupported thumbnail mimetype {}",
                mimetype
            )))
        }
    })
}

/// Returns the FilterType enum value for the given filter name.
/// The matching is case insensitive.
fn parse_filter_type(value: &str) -> Option<FilterType> {
    match value.to_lowercase().as_str() {
        "lanczos3" => Some(FilterType::Lanczos3),
        "triangle" => Some(FilterType::Triangle),
        "catmullrom" => Some(FilterType::CatmullRom),
        "gaussian" => Some(FilterType::Gaussian),
        "nearest" => Some(FilterType::Nearest),
        _ => None,
    }
}

/// Downloads the thumbnail image from the given url and
/// returns the response body as a DynamicImage object.
async fn get_image_from_url(url: &str) -> Result<DynamicImage, Error> {
    // Start the HTTP request and wait for the response.
    let res = reqwest::get(url).await?;
    // Check the response status code before starting the upload.
    if !res.status().is_success() {
        // Non-2xx status code.
        return Err(Error::ThumbnailDownloadError {
            status_code: res.status().as_u16(),
        });
    }
    // Determine the format with the response mimetype header.
    let source_format = mimetype_to_format(
        res.headers()
            .get("content-type")
            .ok_or_else(|| {
                Error::UserInputError(
                    "thumbnail response is missing content-type header".to_owned(),
                )
            })?
            .to_str()
            .unwrap(),
    )?;
    // Decode the image from the response body.
    Ok(image::load_from_memory_with_format(
        res.bytes().await?.as_ref(),
        source_format,
    )?)
}

/// Downloads the thumbnail to the destination bucket.
async fn download_thumbnail(
    metadata: &serde_json::Value,
    options: ThumbnailOptions,
    bucket: Bucket,
    key: String,
) -> Result<(), Error> {
    // Get the thumbnail URL from the info json.
    let thumbnail_url = get_thumbnail_url(metadata)?;
    println!(
        "Downloading thumbnail {} -> s3://{}/{}",
        &thumbnail_url, &bucket.name, &key
    );
    // Download and parse the thumbnail image.
    let img = get_image_from_url(&thumbnail_url).await?;
    // Resize the image if necessary.
    let img = resize_image(img, options.filter, options.width, options.height);
    // Save the image to a temporary file.
    let out_path = format!("/tmp/{}", key);
    img.save_with_format(&out_path, options.format)?;
    defer! {
        // Garbage collect the temporary file.
        let _ = std::fs::remove_file(&out_path);
    }
    let status_code = {
        // Only keep the file open for the duration of the upload.
        let mut body = fs::File::open(&out_path).await?;
        // Stream the file contents to S3.
        bucket.put_object_stream(&mut body, &key).await?
    };
    if status_code != 200 {
        return Err(Error::S3UploadError { status_code });
    }
    println!("Thumbnail download completed successfully");
    Ok(())
}

/// Resizes the image using the specified filter and dimensions.
/// If only one dimension is specified, the other dimension is
/// calculated to maintain the aspect ratio.
fn resize_image(
    img: DynamicImage,
    filter: FilterType,
    width: Option<u32>,
    height: Option<u32>,
) -> DynamicImage {
    match (width, height) {
        // Resize both dimensions to the exact specified size.
        (Some(width), Some(height)) => img.resize(width, height, filter),
        // Resize the width to the specified size and maintain the
        // aspect ratio.
        (Some(width), None) => {
            let aspect = img.width() as f32 / img.height() as f32;
            let height = (width as f32 / aspect) as u32;
            img.resize(width, height, filter)
        }
        // Resize the height to the specified size and maintain the
        // aspect ratio.
        (None, Some(height)) => {
            let aspect = img.width() as f32 / img.height() as f32;
            let width = (height as f32 * aspect) as u32;
            img.resize(width, height, filter)
        }
        // Don't resize the image.
        (None, None) => img,
    }
}

/// Returns the ImageFormat enum value based on the file extension
/// of the given filename/path.
fn get_format_from_filename(filename: &str) -> Option<ImageFormat> {
    Path::new(filename)
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ImageFormat::from_extension)
}

/*
/// Downloads the thumbnail and uploads it to the specified output
/// without doing any conversion. This is optimal performance-wise
/// but does not guarantee the thumbnail will be in the desired
/// format.
/// Remember to import the required traits:
/// ```rust
///     use futures::TryStreamExt;
/// ```
async fn download_raw_thumbnail(
    thumbnail_url: &str,
    bucket: Bucket,
    key: String,
) -> Result<(), Error> {
    let res = reqwest::get(thumbnail_url).await?;
    // Check the response status code before starting the upload.
    if !res.status().is_success() {
        // Non-2xx status code.
        return Err(Error::ThumbnailDownloadError {
            status_code: res.status().as_u16(),
        });
    }
    // Convert the response body to a tokio::ioAsyncRead.
    let mut body = to_tokio_async_read(
        // Use reqwest's stream reader extension.
        res.bytes_stream()
            // Map the error to an io::Error, which is required by AsyncRead.
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            // Convert the stream to a futures::io::AsyncRead.
            .into_async_read(),
    );
    // Upload the response stream to the output bucket.
    let status_code = bucket.put_object_stream(&mut body, &key).await?;
    if status_code != 200 {
        return Err(Error::S3UploadError { status_code });
    }
    println!("thumbnail download completed successfully");
    Ok(())
}

/// Patch for converting a hyper/reqwest response body to a tokio AsyncRead.
/// Source: https://stackoverflow.com/questions/60964238/how-to-write-a-hyper-response-body-to-a-file
fn to_tokio_async_read(r: impl futures::io::AsyncRead) -> impl tokio::io::AsyncRead {
    tokio_util::compat::FuturesAsyncReadCompatExt::compat(r)
}
*/
