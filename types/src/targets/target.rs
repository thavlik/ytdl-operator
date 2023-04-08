use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::*;

/// A reference to a target resource in the same namespace.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct TargetRef {
    /// Kind of the target resource, e.g. `"WebhookTarget"`.
    pub kind: String,

    /// Name of the target resource.
    pub name: String,
}

/// High-level configuration for [`Download`] output. This resource describess
/// which target resources will be used to store the metadata, AV files, and
/// thumbnails.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "Target",
    plural = "targets",
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
pub struct TargetSpec {
    /// List of references to target resources that will be used to store the metadata json.
    pub metadata: Option<Vec<TargetRef>>,

    /// List of references to target resources that will be used to store the AV files.
    pub audiovisual: Option<Vec<TargetRef>>,

    /// List of references to target resources that will be used to store the thumbnail files.
    pub thumbnail: Option<Vec<TargetRef>>,
}
