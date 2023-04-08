use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Specification for the [`DownloadChildProcess`] custom resource, which are created
/// by the [`Download`] controller for each line in the query's metadata jsonl. This
/// way individual videos are downloaded using different IP addresses and overall
/// download speed can scale horizontally with the Kubernetes cluster.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "DownloadChildProcess",
    plural = "downloadchildprocesses",
    status = "DownloadChildProcessStatus",
    namespaced
)]
#[kube(derive = "PartialEq")]
#[kube(derive = "Default")]
#[kube(shortname = "dcp")]
#[kube(shortname = "dlcp")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
pub struct DownloadChildProcessSpec {
    /// Metadata json from `youtube-dl -j`. Populated by the parent
    /// [`Download`] resource upon creation. youtube-dl accepts a
    /// `--load-info-json` argument to load metadata from a file,
    /// so this field is ultimately used to avoid re-querying when
    /// the metadata was already queried by the parent [`Download`].
    pub metadata: String,

    /// Name reference to a `ContentStorage` resource. Inherited from
    /// the parent [`DownloadSpec::output`].
    pub output: String,
}

/// Status object for the [`DownloadChildProcess`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct DownloadChildProcessStatus {
    /// A short description of the [`DownloadChildProcess`] resource's current state.
    pub phase: Option<DownloadChildProcessPhase>,

    /// A human-readable message indicating details about why the
    /// [`DownloadChildProcess`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the download pod was started. Because a [`DownloadChildProcess`]
    /// may be delayed waiting for a VPN slot, this timestamp may be later than the
    /// [`creationTimestamp`](DownloadChildProcess::metadata.creationTimestamp).
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,

    /// Timestamp of when the [`DownloadChildProcessStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// A short description of the [`DownloadChildProcess`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum DownloadChildProcessPhase {
    /// The [`DownloadChildProcess`] resource first appeared to the controller.
    Pending,

    /// The [`DownloadChildProcess`]'s child [`Mask`](vpn_types::Mask) resource is in the
    /// [`Waiting`](vpn_types::MaskPhase::Waiting) phase.
    Waiting,

    /// The [`DownloadChildProcess`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) is being created.
    Starting,

    /// The [`DownloadChildProcess`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) is running.
    Running,

    /// The [`DownloadChildProcess`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) has completed.
    /// This indicates that all content associated with the video (audiovisual, thumbnail,
    /// metadata) is now in storage.
    Succeeded,

    /// The [`DownloadChildProcess`]'s child [`Pod`](k8s_openapi::api::core::v1::Pod) has failed.
    /// The failure could originate from either the child [`Mask`](vpn_types::Mask) or
    /// the child [`Pod`](k8s_openapi::api::core::v1::Pod).
    Failed,
}

impl FromStr for DownloadChildProcessPhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(DownloadChildProcessPhase::Pending),
            "Waiting" => Ok(DownloadChildProcessPhase::Waiting),
            "Starting" => Ok(DownloadChildProcessPhase::Starting),
            "Running" => Ok(DownloadChildProcessPhase::Running),
            "Succeeded" => Ok(DownloadChildProcessPhase::Succeeded),
            "Failed" => Ok(DownloadChildProcessPhase::Failed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for DownloadChildProcessPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadChildProcessPhase::Pending => write!(f, "Pending"),
            DownloadChildProcessPhase::Waiting => write!(f, "Waiting"),
            DownloadChildProcessPhase::Starting => write!(f, "Starting"),
            DownloadChildProcessPhase::Running => write!(f, "Running"),
            DownloadChildProcessPhase::Succeeded => write!(f, "Succeeded"),
            DownloadChildProcessPhase::Failed => write!(f, "Failed"),
        }
    }
}
