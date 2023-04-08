use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Configuration for a target's credentials verification. The controller
/// probes the relevant service to ensure that the credentials are valid
/// before the target enters the `Ready` phase.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct TargetVerifySpec {
    /// If `true`, credentials verification will be bypassed.
    /// Default is `false`.
    pub skip: Option<bool>,

    /// Interval for re-verifying the credentials after they have been
    /// verified for the first time. If unset, the credentials will
    /// only be verified once.
    pub interval: Option<String>,
}

/// Status object for the target resources.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct TargetStatus {
    /// A short description of the resource's current state.
    pub phase: Option<TargetPhase>,

    /// A human-readable message indicating details about why the resource is in this phase.
    pub message: Option<String>,

    /// Timestamp of when this status object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,

    /// Timestamp of when verification last succeeded.
    #[serde(rename = "lastVerified")]
    pub last_verified: Option<String>,
}

/// A short description of the target resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum TargetPhase {
    /// The target resource first appeared to the controller.
    Pending,

    /// The controller is testing the service with the credentials to ensure they are valid.
    Verifying,

    /// The target's backing service is ready to be used.
    Ready,

    /// The credentials test failed with an error.
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
