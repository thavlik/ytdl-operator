use awsregion::Region;
use k8s_openapi::api::core::v1::{PodStatus, Secret};
use kube::{
    api::{Api, ObjectMeta, PostParams, Resource},
    Client, ResourceExt,
};
use s3::{bucket::Bucket, creds::Credentials};
use tokio::time::Duration;
use ytdl_types::*;

pub mod pod;

mod error;

pub use error::Error;

/// Reconciliation return value to requeue the resource immediately.
pub const IMMEDIATELY: Duration = Duration::ZERO;

/// Default S3 region.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Default output key template.
pub const DEFAULT_TEMPLATE: &str = "%(id)s.%(ext)s";

/// Default image to use for the executor. The executor
/// image is responsible for downloading the video and
/// thumbnail from the video service, and uploading them
/// to the storage backend in the desired formats.
pub const DEFAULT_EXECUTOR_IMAGE: &str = "thavlik/ytdl-executor:latest";

/// Key in the ConfigMap for the metadata/info jsonl.
pub const INFO_JSONL_KEY: &str = "info.jsonl";

/// A tuple containing an S3 Bucket and key, which is the
/// final output specification for videos and thumbnails.
/// The spec is ultimately resolved into this object.
pub type Output = (Bucket, String);

/// Creates a child DownloadJob resource for the given Entity.
pub async fn create_executor(
    client: Client,
    instance: &Download,
    id: String,
    metadata: String,
) -> Result<(), Error> {
    let executor = get_entity_executor(instance, id, metadata);
    let api: Api<DownloadJob> = Api::namespaced(client, instance.namespace().as_ref().unwrap());
    api.create(&PostParams::default(), &executor).await?;
    Ok(())
}

pub fn get_executor_service_account_name() -> Result<String, Error> {
    Ok(std::env::var("EXECUTOR_SERVICE_ACCOUNT_NAME")?)
}

/// Returns the phase of the Download
pub fn get_download_phase(instance: &Download) -> Result<DownloadPhase, Error> {
    Ok(instance.status.as_ref().unwrap().phase.unwrap())
}

/// Returns the phase of the DownloadJob.
pub fn get_executor_phase(instance: &DownloadJob) -> Result<DownloadJobPhase, Error> {
    Ok(instance.status.as_ref().unwrap().phase.unwrap())
}

/// Returns the Bucket to be used for video file storage.
pub async fn get_video_output(
    client: Client,
    metadata: &serde_json::Value,
    instance: &DownloadJob,
) -> Result<Option<Output>, Error> {
    let video = match instance.spec.output.video {
        Some(ref video) => video,
        None => return Ok(None),
    };
    let s3 = match video.s3 {
        Some(ref s3) => s3,
        None => return Ok(None),
    };
    let output =
        output_from_spec(client, instance.namespace().as_ref().unwrap(), metadata, s3).await?;
    Ok(Some(output))
}

/// Returns the Bucket to be used for thumbnail storage.
pub async fn get_thumbnail_output(
    client: Client,
    metadata: &serde_json::Value,
    instance: &DownloadJob,
) -> Result<Option<Output>, Error> {
    let thumbnail = match instance.spec.output.thumbnail {
        Some(ref thumbnail) => thumbnail,
        None => return Ok(None),
    };
    let s3 = match thumbnail.s3 {
        Some(ref s3) => s3,
        None => return Ok(None),
    };
    let output =
        output_from_spec(client, instance.namespace().as_ref().unwrap(), metadata, s3).await?;
    Ok(Some(output))
}

/// Returns the S3 Bucket and key template for the given S3OutputSpec.
/// The metadata / info json must be provided to replace the template
/// variables with their values. The kubeclient and namespace are
/// required for retrieving credentials.
async fn output_from_spec(
    client: Client,
    namespace: &str,
    metadata: &serde_json::Value,
    output_spec: &S3OutputSpec,
) -> Result<Output, Error> {
    // Build the S3 Bucket object for uploading.
    let region = get_s3_region(output_spec)?;
    let credentials = get_s3_creds(client, namespace, output_spec).await?;
    let bucket = Bucket::new(&output_spec.bucket, region, credentials)?;
    // Use the default template if none is specified.
    let template = match output_spec.key {
        Some(ref key) => key.clone(),
        None => DEFAULT_TEMPLATE.to_owned(),
    };
    // Convert the template into the actual S3 object key.
    let key = template_key(metadata, &template)?;
    Ok((bucket, key))
}

/// Returns the output key given the template and the
/// video's metadata. This requires deserializing the
/// metadata and iterating over its contents to replace
/// the template variables with their values.
fn template_key(metadata: &serde_json::Value, template: &str) -> Result<String, Error> {
    // Parse the metadata into a generic json object.
    let metadata = metadata
        .as_object()
        .ok_or_else(|| Error::UserInputError("metadata must be a json object".to_owned()))?;
    // Iterate over the key-value pairs and replace the template variables.
    let mut result = template.to_owned();
    for (key, value) in metadata {
        if result.find("%").is_none() {
            // No more template variables to replace; stop early.
            break;
        }
        // Format the key as it would appear in the template.
        let key = format!("%({})s", key);
        // Default to an empty string if the value is not a string.
        let value = value.as_str().unwrap_or("");
        // Replace the template variable with the value.
        result = result.replace(&key, value);
    }
    if result.find("%").is_some() {
        // There are still template variables that were not replaced.
        // This is guaranteed to result in an invalid S3 object key.
        // https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-keys.html
        return Err(Error::UserInputError(
            "metadata does not contain all template variables".to_owned(),
        ));
    }
    Ok(result)
}

/// Returns the S3 credentials for the given S3OutputSpec.
async fn get_s3_creds(
    client: Client,
    namespace: &str,
    spec: &S3OutputSpec,
) -> Result<Credentials, Error> {
    match spec.secret {
        Some(ref secret) => {
            let api: Api<Secret> = Api::namespaced(client, namespace);
            let secret = api.get(secret).await?;
            let access_key_id = get_secret_value(&secret, "access_key_id")?;
            let secret_access_key = get_secret_value(&secret, "secret_access_key")?;
            let security_token = get_secret_value(&secret, "security_token")?;
            let session_token = get_secret_value(&secret, "session_token")?;
            Ok(Credentials::new(
                access_key_id.as_deref(),
                secret_access_key.as_deref(),
                security_token.as_deref(),
                session_token.as_deref(),
                None, // expiration
            )?)
        }
        None => Ok(Credentials::default()?),
    }
}

/// Returns the secret value for the given key.
/// This requires an allocation because it's unclear
/// how to pass &ByteString into std::str::from_utf8
/// and still satisfy the borrow checker.
fn get_secret_value(secret: &Secret, key: &str) -> Result<Option<String>, Error> {
    Ok(match secret.data {
        Some(ref data) => match data.get(key) {
            Some(s) => Some(serde_json::to_string(s)?),
            None => None,
        },
        None => None,
    })
}

/// Returns the S3 Region object for the given S3OutputSpec.
fn get_s3_region(spec: &S3OutputSpec) -> Result<Region, Error> {
    let region = match spec.region.as_ref() {
        // Use the region from the spec.
        Some(region) => region.to_owned(),
        // Use the default region.
        None => DEFAULT_REGION.to_owned(),
    };
    Ok(match spec.endpoint.as_ref() {
        // Custom endpoint support (e.g. https://nyc3.digitaloceanspaces.com)
        Some(endpoint) => Region::Custom {
            region,
            endpoint: endpoint.clone(),
        },
        // The Region object is based solely on the region name.
        None => region.parse()?,
    })
}

pub fn check_pod_scheduling_error(status: &PodStatus) -> Option<String> {
    let conditions: &Vec<_> = match status.conditions.as_ref() {
        Some(conditions) => conditions,
        None => return None,
    };
    for condition in conditions {
        if condition.type_ == "PodScheduled" && condition.status == "False" {
            return Some(
                condition
                    .message
                    .as_deref()
                    .unwrap_or("PodScheduled == False, but no message was provided")
                    .to_owned(),
            );
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Entity {
    pub id: String,
    pub metadata: String,
}

/// Returns an DownloadJob owned by the Download resource that
/// is configured for the Entity.
pub fn get_entity_executor(instance: &Download, id: String, metadata: String) -> DownloadJob {
    // Make the Download the owner of the DownloadJob.
    let oref = instance.controller_owner_ref(&()).unwrap();
    DownloadJob {
        metadata: ObjectMeta {
            name: Some(format!("{}-{}", instance.name_any(), id)),
            namespace: Some(instance.namespace().unwrap()),
            owner_references: Some(vec![oref]),
            ..Default::default()
        },
        spec: DownloadJobSpec {
            // The DownloadJob's metadata is the Entity's metadata.
            metadata,
            // Inherit the Download's executor image.
            executor: instance.spec.executor.clone(),
            // Inherit the Download's extra arguments.
            extra: instance.spec.extra.clone(),
            // Inherit the Download's output spec.
            output: instance.spec.output.clone(),
        },
        ..Default::default()
    }
}

/// Returns the [`DownloadJob`] with the given name/namespace.
pub async fn get_download_job(
    client: Client,
    name: &str,
    namespace: &str,
) -> Result<Option<DownloadJob>, Error> {
    match Api::<DownloadJob>::namespaced(client, namespace)
        .get(&name)
        .await
    {
        Ok(dj) => Ok(Some(dj)),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}
