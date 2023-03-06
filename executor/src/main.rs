use clap::Parser;
use futures::TryStreamExt;
use kube::client::Client;
use s3::bucket::Bucket;
use tokio::io::BufReader;
use tokio::process::Command;
use ytdl_common::{get_thumbnail_output, get_video_output, Error};
use ytdl_types::Executor;
mod ready;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value_t = false)]
    download_video: bool,

    #[arg(long, default_value_t = false)]
    download_thumbnail: bool,
}

fn get_command() -> String {
    std::env::var("YOUTUBE_DL_COMMAND").unwrap_or_else(|_| "youtube-dl".to_owned())
}

#[tokio::main]
async fn main() {
    // Prepare the environment first so we can fail early.
    println!("preparing download environment");
    let client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");
    let args = Args::parse();
    let instance: Executor =
        get_resource().expect("failed to get Executor resource from environment");

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
    println!("environment prepared, waiting for VPN to connect");
    ready::wait_for_vpn().await.expect("vpn failed to connect");
    println!("VPN is connected");

    // Start the download(s).
    match outputs {
        (Some(video_output), Some(thumbnail_output)) => {
            println!("downloading video and thumbnail");
            let result = tokio::join!(
                download_video(&metadata, video_output.0, video_output.1, &command, extra),
                download_thumbnail(&metadata, thumbnail_output.0, thumbnail_output.1),
            );
            result.0.expect("failed to download video");
            result.1.expect("failed to download thumbnail");
        }
        (Some(video_output), None) => {
            println!("downloading video");
            download_video(&metadata, video_output.0, video_output.1, &command, extra)
                .await
                .expect("failed to download video");
        }
        (None, Some(thumbnail_output)) => {
            println!("downloading thumbnail");
            download_thumbnail(&metadata, thumbnail_output.0, thumbnail_output.1)
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

fn get_resource() -> Result<Executor, Error> {
    Ok(serde_json::from_str(&std::env::var("RESOURCE")?)?)
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
fn build_cmd(command: &str, webpage_url: &str, extra: Option<&str>) -> String {
    let mut cmd = format!("{} -o -", command);
    if let Some(extra) = extra {
        cmd = format!("{} {}", cmd, extra);
    }
    format!("{} -- {}", cmd, webpage_url)
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
        "downloading video {} -> s3://{}/{}",
        webpage_url, &bucket.name, &key
    );
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&build_cmd(command, webpage_url, extra))
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::GenericError("failed to get child process stdout".to_owned()))?;
    let mut reader = BufReader::new(stdout);
    let status_code = bucket.put_object_stream(&mut reader, &key).await?;
    if status_code != 200 {
        return Err(Error::S3UploadError { status_code });
    }
    let status = child.wait().await?;
    if status.success() {
        // Upload completed and youtube-dl exited successfully.
        println!("video download completed successfully");
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

/// Downloads the thumbnail and uploads it to the specified output.
async fn download_thumbnail(
    metadata: &serde_json::Value,
    bucket: Bucket,
    key: String,
) -> Result<(), Error> {
    let thumbnail_url = get_thumbnail_url(metadata)?;
    println!(
        "downloading thumbnail {} -> s3://{}/{}",
        &thumbnail_url, &bucket.name, &key
    );
    let res = reqwest::get(thumbnail_url).await?;
    // Check the response status code before starting the upload.
    if !res.status().is_success() {
        // Non-2xx status code.
        return Err(Error::ThumbnailDownloadError {
            status_code: res.status().as_u16(),
        });
    }
    //
    // TODO: verify mimetype? (see put_object_stream_with_mimetype)
    // TODO: normalize the image format?
    //
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
