use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

use image_format::ImageFormat;
use image_filter::ImageFilter;

/// S3-compatiable storage configuration.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct S3OutputSpec {
    /// S3 bucket name (required).
    pub bucket: String,

    /// S3 object key template. Refer to youtube-dl documentation
    /// for details on which keys are available:
    /// <https://github.com/ytdl-org/youtube-dl#output-template>.
    /// The default value is `"%(id)s.%(ext)s"`.
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
}

/// Configuration for audiovisual content storage.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct VideoStorageSpec {
    /// Download video format, injected as `youtube-dl`'s `--format`
    /// option. Refer to the youtube-dl documentation:
    /// <https://github.com/ytdl-org/youtube-dl/blob/master/README.md#format-selection>
    /// 
    /// Defaults to `"best"`.
    /// 
    /// It is highly recommended to specify the format so it will be
    /// consistent across all videos, but the ability to download the
    /// highest quality video regardless of the format is maintained
    /// for niche purposes. If two platforms use different formats for
    /// their highest quality videos, you should create two `ContentStorage`
    /// resources that each specify the best format for each platform,
    /// as opposed to creating a single `ContentStorage` to receive mixed
    /// format videos.
    pub format: Option<String>,

    /// Amazon S3-compatible output.
    pub s3: S3OutputSpec,
}

/// Configuration for metadata storage.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct MetadataStorageSpec {
    /// Amazon S3-compatible output.
    pub s3: S3OutputSpec,
}

/// Configuration for thumbnail storage.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct ThumbnailStorageSpec {
    /// Image format (`jpg`, `png`, etc.) The thumbnail will be converted
    /// to conform to this format. If unspecified, the image is not
    /// converted. See the crate [`image-convert`](https://crates.io/crates/image-convert).
    pub format: Option<ImageFormat>,

    /// Resize width. If specified, the thumbnail will be resized
    /// to this width. If height is also specified, the thumbnail
    /// will be resized to fit within the specified dimensions,
    /// otherwise the aspect ratio is maintained.
    pub width: Option<u32>,

    /// Resize height. If specified, the thumbnail will be resized
    /// to this height. If width is also specified, the thumbnail
    /// will be resized to fit within the specified dimensions,
    /// otherwise the aspect ratio is maintained.
    pub height: Option<u32>,

    /// Image filter algorithm to use when resizing.
    pub filter: Option<ImageFilter>,

    /// Amazon S3-compatible output.
    pub s3: S3OutputSpec,
}

/// Struct corresponding to the Specification (`spec`) part of the `ContentStorage` resource.
/// The `ContentStorage` custom resource is responsible for configuring the storage of
/// audiovisual content, metadata json, and thumbnail images.
/// 
/// The same `ContentStorage` resource can be referenced by multiple `Download` resources
/// to unify the storage configuration for multiple downloads. This way, the configuration
/// is all in one place, and updating it is trivial regardless of how many downloads
/// are running.
/// 
/// Currently, everything is stored in S3-compatible buckets. If you require alternative storage
/// means, please open an issue or consider using [NooBaa](https://www.noobaa.io/) as a proxy
/// between ytdl-operator and your storage backend.
#[derive(CustomResource, Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "ContentStorage",
    plural = "downloads",
    derive = "PartialEq",
    status = "ContentStorageStatus",
    namespaced
)]
#[kube(derive = "Default")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
pub struct ContentStorageSpec {
    /// Audiovisual content output specification. Configure this field to
    /// download audio, video, or both to an S3 bucket.
    pub video: Option<VideoStorageSpec>,

    /// Metadata output specification. Configure this field to cache the
    /// video info json in an S3 bucket.
    pub metadata: Option<MetadataStorageSpec>,

    /// Thumbnail output specification. Configure this field to cache the
    /// video thumbnails in an S3 bucket.
    pub thumbnail: Option<ThumbnailStorageSpec>,
}

/// Status object for the [`ContentStorage`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct ContentStorageStatus {
    /// A short description of the [`ContentStorage`] resource's current state.
    pub phase: Option<ContentStoragePhase>,

    /// A human-readable message indicating details about why the
    /// [`ContentStorage`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the [`ContentStorageStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// A short description of the [`ContentStorage`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum ContentStoragePhase {
    /// The [`ContentStorage`] resource first appeared to the controller.
    Pending,

    /// No issues detected with the [`ContentStorage`] resource.
    Healthy,
}

impl FromStr for ContentStoragePhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(ContentStoragePhase::Pending),
            "Healthy" => Ok(ContentStoragePhase::Healthy),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ContentStoragePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentStoragePhase::Pending => write!(f, "Pending"),
            ContentStoragePhase::Healthy => write!(f, "Healthy"),
        }
    }
}