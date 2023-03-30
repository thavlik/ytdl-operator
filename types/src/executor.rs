use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// A short description of the [`Executor`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum ExecutorPhase {
    /// The [`Executor`] resource first appeared to the controller.
    Pending,

    /// The [`Executor`]'s child [`Mask`] resource has the
    /// [`MaskPhase::Waiting`](vpn_types::MaskPhase::Waiting) status.
    Waiting,

    /// The [`Executor`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) is being created.
    Starting,

    /// The [`Executor`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) is running.
    Running,

    /// The [`Executor`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) has completed.
    Succeeded,
    
    /// The [`Executor`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) has failed.
    Failed,
}

impl FromStr for ExecutorPhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(ExecutorPhase::Pending),
            "Waiting" => Ok(ExecutorPhase::Waiting),
            "Starting" => Ok(ExecutorPhase::Starting),
            "Downloading" => Ok(ExecutorPhase::Downloading),
            "Succeeded" => Ok(ExecutorPhase::Succeeded),
            "Failed" => Ok(ExecutorPhase::Failed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ExecutorPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutorPhase::Pending => write!(f, "Pending"),
            ExecutorPhase::Waiting => write!(f, "Waiting"),
            ExecutorPhase::Starting => write!(f, "Starting"),
            ExecutorPhase::Downloading => write!(f, "Downloading"),
            ExecutorPhase::Succeeded => write!(f, "Succeeded"),
            ExecutorPhase::Failed => write!(f, "Failed"),
        }
    }
}

/// Struct corresponding to the Specification (`spec`) part of the `Executor` resource. Directly
/// reflects context of the `crds/ytdl.beebs.dev_executor_crd.yaml` file to be found in this repository.
/// The `Executor` struct will be generated by the `CustomResource` derive macro.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "Executor",
    plural = "executors",
    derive = "PartialEq",
    status = "ExecutorStatus",
    namespaced
)]
#[kube(derive = "Default")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
pub struct ExecutorSpec {
    /// Metadata json from `youtube-dl -j`. Populated by the parent
    /// [`Download`] resource upon creation. youtube-dl accepts a
    /// `--load-info-json` argument to load metadata from a file,
    /// so this field is ultimately used to avoid re-querying metadata.
    pub metadata: String,

    /// Overrides inherited from the parent [`DownloadSpec::overrides`].
    /// Use this member to override the `ytdl-executor` image or the
    /// arguments passed to `youtube-dl`.
    pub overrides: Option<ExecutorOverridesSpec>,

    /// Output specification. Once the metadata is queried,
    /// the output specification will be used to download
    /// the videos, metadata, and/or thumbnails to the
    /// configured storage backend(s).
    pub output: OutputSpec,
}

/// Status object for the [`Executor`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct ExecutorStatus {
    /// A short description of the [`Executor`] resource's current state.
    pub phase: Option<ExecutorPhase>,

    /// A human-readable message indicating details about why the
    /// [`Executor`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the download pod was started.
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,

    /// Timestamp of when the [`ExecutorStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}
