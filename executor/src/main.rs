use clap::Parser;
use image::{imageops::FilterType, DynamicImage, ImageFormat};
use kube::client::Client;
use s3::bucket::Bucket;
use scopeguard::defer;
use std::{env, ffi::OsStr, path::Path, process::Stdio};
use tokio::process::Command;
use tokio::{fs, io::BufReader};
use ytdl_common::{get_thumbnail_output, get_video_output, Error};
use ytdl_types::{Executor, ThumbnailStorageSpec};

mod ready;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value_t = false)]
    download_video: bool,

    #[arg(long, default_value_t = false)]
    download_thumbnail: bool,
}

/// Returns the precise youtube-dl command to use,
/// which may be overriden to use e.g. yt-dlp
fn get_command() -> String {
    env::var("YOUTUBE_DL_COMMAND").unwrap_or_else(|_| "youtube-dl".to_owned())
}

/// Path for the metadata info json file. youtube-dl can only
/// load this from a file, and it's convenient to write it out
/// for debugging purposes (e.g. `cat /info.json`).
const INFO_JSON_PATH: &str = "/info.json";

#[tokio::main]
async fn main() {
    // Prepare the environment first so we can fail early.
    println!("Preparing download environment");
    let client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");
    let args = Args::parse();
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

    // Get the youtube-dl command to use from the spec.
    let command = get_command();

    // Get the extra args from the spec.
    let extra: Option<&str> = instance.spec.extra.as_deref();

    // Determine what we need to do, download-wise, and
    // get the output objects at the same time.
    let outputs = get_outputs(
        client,
        &metadata,
        &instance,
        args.download_video,
        args.download_thumbnail,
    )
    .await
    .expect("failed to get outputs");

    // Wait for the VPN to connect before starting the download.
    println!("Environment prepared, waiting for VPN to connect");
    ready::wait_for_vpn().await.expect("vpn failed to connect");
    println!("VPN is connected");

    // Start the download(s).
    match outputs {
        (Some(video_output), Some(thumbnail_output)) => {
            let thumbnail_opts = get_thumbnail_opts(&instance, &thumbnail_output.1)
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
        (Some(video_output), None) => {
            println!("Downloading video");
            download_video(&metadata, video_output.0, video_output.1, &command, extra)
                .await
                .expect("failed to download video");
        }
        (None, Some(thumbnail_output)) => {
            let thumbnail_opts = get_thumbnail_opts(&instance, &thumbnail_output.1)
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
struct ThumbnailOptions {
    format: ImageFormat,
    filter: FilterType,
    width: Option<u32>,
    height: Option<u32>,
}

/// Returns a struct containing processing options for the thumbnail.
fn get_thumbnail_opts(instance: &Executor, key: &str) -> Result<ThumbnailOptions, Error> {
    let thumbnail: &ThumbnailStorageSpec = instance.spec.output.thumbnail.as_ref().unwrap();
    // Determine the sampling filter to use when resizing.
    let filter = match thumbnail.filter {
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
    let format = match thumbnail.format {
        // Prefer the overridden format in the spec.
        Some(ref format) => ImageFormat::from_extension(format).ok_or_else(|| {
            Error::UserInputError(format!("unsupported thumbnail format: {}", format))
        })?,
        // Default to the format inferred from the output key.
        None => match get_extension_from_filename(key) {
            Some(format) => format,
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
const VIDEO_OUTPUT_MISSING: &str = "video output requested but no output spec provided";

/// Error code for missing thumbnail output spec. The operator
/// should never ask an Executor pod to download a thumbnail
/// without providing an output spec. This is considered
/// an unreachable error.
const THUMBNAIL_OUTPUT_MISSING: &str = "thumbnail output requested but no output spec provided";

/// Returns the output objects for the executor.
async fn get_outputs(
    client: Client,
    metadata: &serde_json::Value,
    instance: &Executor,
    download_video: bool,
    download_thumbnail: bool,
) -> Result<(Option<(Bucket, String)>, Option<(Bucket, String)>), Error> {
    match (download_video, download_thumbnail) {
        (true, true) => {
            let result = tokio::join!(
                get_video_output(client.clone(), &metadata, &instance),
                get_thumbnail_output(client.clone(), &metadata, &instance),
            );
            let video_output = result.0?.expect(VIDEO_OUTPUT_MISSING);
            let thumbnail_output = result.1?.expect(THUMBNAIL_OUTPUT_MISSING);
            Ok((Some(video_output), Some(thumbnail_output)))
        }
        (true, false) => {
            let video_output = get_video_output(client, metadata, instance)
                .await?
                .expect(VIDEO_OUTPUT_MISSING);
            Ok((Some(video_output), None))
        }
        (false, true) => {
            let thumbnail_output = get_thumbnail_output(client, metadata, instance)
                .await?
                .expect(THUMBNAIL_OUTPUT_MISSING);
            Ok((None, Some(thumbnail_output)))
        }
        (false, false) => Ok((None, None)),
    }
}

/// Builds the AV download command for youtube-dl.
fn build_cmd(command: &str, extra: Option<&str>) -> String {
    let mut cmd = format!("{} --load-info-json {} -o -", command, INFO_JSON_PATH);
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

fn mimetype_to_format(mimetype: &str) -> Result<ImageFormat, Error> {
    match mimetype {
        "image/jpeg" => Ok(ImageFormat::Jpeg),
        "image/png" => Ok(ImageFormat::Png),
        "image/gif" => Ok(ImageFormat::Gif),
        "image/webp" => Ok(ImageFormat::WebP),
        "image/tiff" => Ok(ImageFormat::Tiff),
        "image/bmp" => Ok(ImageFormat::Bmp),
        "image/x-icon" => Ok(ImageFormat::Ico),
        _ => Err(Error::UserInputError(format!(
            "unsupported thumbnail mimetype {}",
            mimetype
        ))),
    }
}

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

async fn get_image_from_url(url: &str) -> Result<DynamicImage, Error> {
    let res = reqwest::get(url).await?;
    // Check the response status code first.
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
    // Load the image from the response body.
    Ok(image::load_from_memory_with_format(
        res.bytes().await?.as_ref(),
        source_format,
    )?)
}

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

fn get_extension_from_filename(filename: &str) -> Option<ImageFormat> {
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
