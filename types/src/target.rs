use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

use crate::common::*;

/// Struct corresponding to the Specification (`spec`) part of the `Target` resource.
/// The `Target` custom resource is responsible for configuring the storage of metadata.
#[derive(CustomResource, Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "Target",
    plural = "metadatastorages",
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
    pub videos: Option<Vec<String>>,
    pub metadata: Option<Vec<String>>,
    pub thumbnails: Option<Vec<String>>,
}

/// Status object for the [`Target`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct TargetStatus {
    /// A short description of the [`Target`] resource's current state.
    pub phase: Option<TargetPhase>,

    /// A human-readable message indicating details about why the
    /// [`Target`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the [`TargetStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// A short description of the [`Target`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum TargetPhase {
    /// The [`Target`] resource first appeared to the controller.
    Pending,

    /// The controller is testing the storage backend service with the
    /// credentials to ensure they are valid.
    Verifying,

    /// The storage backend is ready to be used.
    Ready,

    /// The credentials test(s) failed with an error.
    ErrVerifyFailed,
}

impl FromStr for TargetPhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(TargetPhase::Pending),
            "Verifying" => Ok(TargetPhase::Verifying),
            "Ready" => Ok(TargetPhase::Ready),
            "ErrVerifyFailed" => Ok(TargetPhase::ErrVerifyFailed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for TargetPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetPhase::Pending => write!(f, "Pending"),
            TargetPhase::Verifying => write!(f, "Verifying"),
            TargetPhase::Ready => write!(f, "Ready"),
            TargetPhase::ErrVerifyFailed => write!(f, "ErrVerifyFailed"),
        }
    }
}
