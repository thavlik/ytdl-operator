use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::*;

/// S3-compatiable storage configuration. All content (video, audio, thumbnail,
/// and metadata json) are stored in S3 buckets. You can use the same bucket
/// for everything or different buckets for the different types of content.
/// If you use the same bucket, it is recommended to prefix the [`key`](S3TargetSpec::key)
/// template with the type of content. This way you can iterate over only the objects
/// you want by setting the `prefix` option in [`ListObjects`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjects.html).
///
/// Examples:
///     - "av/%(id)s.%(ext)s" becomes "av/sn77AsWwTnU.webm" for the audiovisual file
///     - "md/%(id)s.%(ext)s" becomes "md/sn77AsWwTnU.json" for the metadata
///     - "thumb/%(id)s.%(ext)s" becomes "thumb/sn77AsWwTnU.jpg" for the thumbnail
///
/// This will allow you to use `prefix="md/"` to list only the metadata.
///
/// If your project requires a different storage backend, consider using
/// [NooBaa](https://www.noobaa.io/) as a proxy between ytdl-operator and
/// the service.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "S3Target",
    plural = "s3targets",
    status = "TargetStatus",
    namespaced
)]
#[kube(derive = "PartialEq")]
#[kube(derive = "Default")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
pub struct S3TargetSpec {
    /// S3 bucket name (required).
    pub bucket: String,

    /// S3 object key template. Refer to youtube-dl documentation
    /// for details on which template variables are available:
    /// <https://github.com/ytdl-org/youtube-dl#output-template>.
    /// The default value is `"%(id)s.%(ext)s"`. `%(ext)s` will be
    /// assigned by the controller in accordance with the relevant
    /// content type, e.g. `json` when storing metadata.
    pub key: Option<String>,

    /// Kubernetes `Secret` resource name containing S3 credentials
    /// as the `access_key_id` and `secret_access_key` fields.
    /// If no credentials are specified, the default creds are used.
    /// This is typical behavior on AWS, but will not be the case
    /// for other S3-compatible backends.
    pub secret: Option<String>,

    /// S3 region. Default is `"us-east-1"`.
    pub region: Option<String>,

    /// Alternative S3 endpoint (e.g. `"https://nyc3.digitaloceanspaces.com"`)
    pub endpoint: Option<String>,

    /// Verification configuration for the S3 service. Default behavior is to
    /// verify the credentials once and never again.
    pub verify: Option<TargetVerifySpec>,
}
