use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Specification for the [`Download`] resource, which is the central custom resource
/// for downloading videos with ytdl-operator. The controller will first query the
/// URL for the info json, then individual pods are created to download each video.
#[derive(CustomResource, Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "Download",
    plural = "downloads",
    status = "DownloadStatus",
    namespaced
)]
#[kube(derive = "PartialEq")]
#[kube(derive = "Default")]
#[kube(shortname = "dl")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
pub struct DownloadSpec {
    /// Input query to youtube-dl. Can be a URL, YouTube video ID, or anything
    /// else accepted as input by `youtube-dl`.
    pub input: String,

    /// If `true`, ignore errors in querying individual entities. This is usually
    /// recommended for playlists and channels because the query will continue
    /// even if some videos are age restricted or otherwise not available.
    /// Set to `false` for single videos or to guarantee that every video for a
    /// playlist/channel is downloaded.
    ///
    /// Equates to the `--ignore-errors` flag in `youtube-dl`.
    #[serde(rename = "ignoreErrors")]
    pub ignore_errors: Option<bool>,

    /// Interval to re-query metadata. This is used to keep a channel or playlist
    /// synchronized after the initial query. Example: `"48h"` will re-query the
    /// input every two days, downloading new videos as they are discovered.
    #[serde(rename = "queryInterval")]
    pub query_interval: Option<String>,

    /// Names of the [`Target`] resources that describe where the different outputs
    /// will be stored. At least one target must be specified.
    pub targets: Vec<String>,
}

/// Status object for the [`Download`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct DownloadStatus {
    /// A short description of the [`Download`] resource's current state.
    pub phase: Option<DownloadPhase>,

    /// A human-readable message indicating details about why the
    /// [`Download`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the [`DownloadStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,

    /// Timestamp of when the query pod started. Because pod creation may
    /// be delayed waiting for a VPN provider, this may be later than the
    /// [`Download`]'s creation timestamp. If [`DownloadSpec::query_interval`]
    /// is specified, this will be the timestamp of when the last query
    /// was started.
    #[serde(rename = "queryStartTime")]
    pub query_start_time: Option<String>,

    /// Timestamp of last metadata query completion. If [`DownloadSpec::query_interval`]
    /// is specified, this is used to determine if the metadata is "stale" and should be
    /// re-queried.
    #[serde(rename = "lastQueried")]
    pub last_queried: Option<String>,

    /// Total number of videos associated with the query. Equivalent to the
    /// count of newlines in the metadata jsonl.
    #[serde(rename = "totalVideos")]
    pub total_videos: Option<u32>,

    /// Number of successfully completed [`DownloadChildProcesses`](DownloadChildProcess),
    /// used to track progress for long-running tasks and gauge how many videos were skipped
    /// due to age restrictions or other errors.
    #[serde(rename = "downloadedVideos")]
    pub downloaded_videos: Option<u32>,
}

/// A short description of the [`Download`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum DownloadPhase {
    /// The [`Download`] resource first appeared to the controller.
    Pending,

    /// The controller is waiting on a [`Mask`](vpn_types::Mask) to proceed
    /// with querying the metadata.
    Waiting,

    /// The metadata is being queried. [`DownloadChildProcess`] resources will be created
    /// for each video as the info is received. Each line of output corresponds
    /// with a [`DownloadChildProcessSpec::metadata`].
    Querying,

    /// One or more [`DownloadChildProcess`] resources are downloading content. The query
    /// may still be in progress, but there is at least one video that is being
    /// downloaded.
    Downloading,

    /// All content has been downloaded successfully. Unless [`DownloadSpec::query_interval`]
    /// is specified, the resource is considered to be in its final state.
    Succeeded,

    /// The query [`Mask`](vpn_types::Mask) or [`Pod`](k8s_openapi::api::core::v1::Pod)
    /// failed with an error. This could be caused by an error with VPN provider assignemnt,
    /// an age restriction error message, or a failure to create the [`ConfigMap`](k8s_openapi::api::core::v1::ConfigMap)
    /// that caches the jsonl metadata.
    ErrQueryFailed,

    /// One or more downloads failed with error(s). This could be from a storage
    /// backend error or if an age restriction error message is received and the
    /// [`DownloadSpec::ignore_errors`] option is `false`.
    ErrDownloadFailed,
}

impl FromStr for DownloadPhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(DownloadPhase::Pending),
            "Waiting" => Ok(DownloadPhase::Waiting),
            "Querying" => Ok(DownloadPhase::Querying),
            "Downloading" => Ok(DownloadPhase::Downloading),
            "Succeeded" => Ok(DownloadPhase::Succeeded),
            "ErrQueryFailed" => Ok(DownloadPhase::ErrQueryFailed),
            "ErrDownloadFailed" => Ok(DownloadPhase::ErrDownloadFailed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for DownloadPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadPhase::Pending => write!(f, "Pending"),
            DownloadPhase::Waiting => write!(f, "Waiting"),
            DownloadPhase::Querying => write!(f, "Querying"),
            DownloadPhase::Downloading => write!(f, "Downloading"),
            DownloadPhase::Succeeded => write!(f, "Succeeded"),
            DownloadPhase::ErrQueryFailed => write!(f, "ErrQueryFailed"),
            DownloadPhase::ErrDownloadFailed => write!(f, "ErrDownloadFailed"),
        }
    }
}
