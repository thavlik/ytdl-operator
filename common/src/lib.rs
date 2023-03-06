use awsregion::Region;
use kube::{
    client::Client, Api, ResourceExt,
};
use k8s_openapi::api::core::v1::Secret;
use s3::{bucket::Bucket, creds::Credentials};
use ytdl_types::{Executor, S3OutputSpec};

mod error;

pub use error::Error;

/// Default S3 region
pub const DEFAULT_REGION: &str = "us-east-1";

/// Default output key template
pub const DEFAULT_TEMPLATE: &str = "%(id)s.%(ext)s";

/// The IP service to use for getting the public IP address.
pub const IP_SERVICE: &str = "https://api.ipify.org";

/// Returns the Bucket to be used for video file storage.
pub async fn get_video_output(
    client: Client,
    metadata: &serde_json::Value,
    instance: &Executor,
) -> Result<Option<(Bucket, String)>, Error> {
    match instance.spec.output.video {
        Some(ref video) => match video.s3 {
            Some(ref spec) => Ok(Some(
                output_from_spec(
                    client,
                    instance.namespace().as_ref().unwrap(),
                    metadata,
                    spec,
                ).await?,
            )),
            None => Ok(None),
        },
        None => Ok(None),
    }
}

/// Returns the Bucket to be used for thumbnail storage.
pub async fn get_thumbnail_output(
    client: Client,
    metadata: &serde_json::Value,
    instance: &Executor,
) -> Result<Option<(Bucket, String)>, Error> {
    match instance.spec.output.thumbnail {
        Some(ref thumbnail) => match thumbnail.s3 {
            Some(ref spec) => Ok(Some(
                output_from_spec(
                    client,
                    instance.namespace().as_ref().unwrap(),
                    metadata,
                    spec,
                ).await?,
            )),
            None => Ok(None),
        },
        None => Ok(None),
    }
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
) -> Result<(Bucket, String), Error> {
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
fn template_key(
    metadata: &serde_json::Value,
    template: &str,
) -> Result<String, Error> {
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
            let secret: Secret = api.get(secret).await?;
            let access_key_id = get_secret_value(&secret, "access_key_id")?;
            let secret_access_key = get_secret_value(&secret, "secret_access_key")?;
            Ok(Credentials::new(
                access_key_id.as_deref(),
                secret_access_key.as_deref(),
                None, // security token
                None, // session token
                None, // profile
            )?)
        },
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
        Some(region) => region.to_owned(),
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
