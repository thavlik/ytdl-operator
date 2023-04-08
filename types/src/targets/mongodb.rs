use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::*;

/// Configuration for MongoDB-compatible metadata output. Use this if your application
/// is designed to retrieved the metadata json from a MongoDB database. The executor
/// pods will connect to the database and insert the metadata json while querying.
/// The executor uses the [`mongodb`](https://crates.io/crates/mongodb) crate to
/// connect to the database and store the metadata json as-is in a single collection.
/// Thumbnails and AV files are stored in the `thumbnails` and `av` collections,
/// respectively. The document's `_id` field is derived the same way as with metadata,
/// and the only other field in the document is `payload` that contains the file bytes.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "MongoDBTarget",
    plural = "mongodbtargets",
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
pub struct MongoDBTargetSpec {
    /// Name of the Kubernetes [`Secret`](https://kubernetes.io/docs/concepts/configuration/secret/)
    /// resource containing the database credentials. The secret must contain
    /// the following fields:
    ///     - `username`
    ///     - `password`
    ///     - `host`
    ///     - `port`
    ///     - `database`
    ///     - `sslmode`
    ///     - `sslcert` (where necessary)
    pub secret: String,

    /// Collection name override. Default depends on the type of content being stored.
    /// For metadata, the default value is `"metadata"`.
    pub collection: Option<String>,

    /// Override template for documents' `_id` field. Refer to the youtube-dl
    /// documentation on output templates:
    /// <https://github.com/ytdl-org/youtube-dl/blob/master/README.md#output-template>
    /// Default value is `"%(id)s"`, which will use the video ID as the document ID.
    /// The rest of the document is the metadata json itself (i.e. the output of
    /// `youtube-dl --dump-json`).
    /// When storing non-metadata, this field must be specified.
    pub id: Option<String>,

    /// Verification settings for the MongoDB database. Default behavior is to
    /// verify the credentials once and never again.
    pub verify: Option<TargetVerifySpec>,
}
