use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

use crate::common::*;

/// Struct corresponding to the Specification (`spec`) part of the `MetadataTarget` resource.
/// The `MetadataTarget` custom resource is responsible for configuring the storage of metadata.
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "MetadataTarget",
    plural = "metadatatargets",
    status = "MetadataTargetStatus",
    namespaced
)]
#[kube(derive = "PartialEq")]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.phase\", \"name\": \"PHASE\", \"type\": \"string\" }"
)]
#[kube(
    printcolumn = "{\"jsonPath\": \".status.lastUpdated\", \"name\": \"AGE\", \"type\": \"date\" }"
)]
#[serde(rename_all = "camelCase")]
#[derive(CustomResource, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
pub enum MetadataTargetSpec {
    /// Amazon S3-compatible output for metadata `.json` files.
    S3(S3OutputSpec),

    /// SQL-compatible output for the metadata. The executor uses the
    /// [`sqlx`](https://crates.io/crates/sqlx) crate to connect to the database,
    /// which supports a wide variety of SQL-compatible databases.
    /// TODO: the schema is currently pending implementation.
    Sql(SqlOutputSpec),

    /// MongoDB-compatible output for the metadata. The executor uses the
    /// [`mongodb`](https://crates.io/crates/mongodb) crate to connect to
    /// the database/cluster.
    #[serde(rename = "mongodb")]
    MongoDB(MongoDBOutputSpec),

    /// Redis cache configuration for the metadata.
    Redis(RedisOutputSpec),
}

/// Status object for the [`MetadataTarget`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct MetadataTargetStatus {
    /// A short description of the [`MetadataTarget`] resource's current state.
    pub phase: Option<MetadataTargetPhase>,

    /// A human-readable message indicating details about why the
    /// [`MetadataTarget`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the [`MetadataTargetStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,

    /// Timestamp of when verification last succeeded.
    #[serde(rename = "lastVerified")]
    pub last_verified: Option<String>,
}

/// A short description of the [`MetadataTarget`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum MetadataTargetPhase {
    /// The [`MetadataTarget`] resource first appeared to the controller.
    Pending,

    /// The controller is testing the metadata storage backend service
    /// with the credentials to ensure they are valid.
    Verifying,

    /// The metadata storage backend is ready to be used.
    Ready,

    /// The credentials test(s) failed with an error.
    ErrVerifyFailed,
}

impl FromStr for MetadataTargetPhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(MetadataTargetPhase::Pending),
            "Verifying" => Ok(MetadataTargetPhase::Verifying),
            "Ready" => Ok(MetadataTargetPhase::Ready),
            "ErrVerifyFailed" => Ok(MetadataTargetPhase::ErrVerifyFailed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for MetadataTargetPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetadataTargetPhase::Pending => write!(f, "Pending"),
            MetadataTargetPhase::Verifying => write!(f, "Verifying"),
            MetadataTargetPhase::Ready => write!(f, "Ready"),
            MetadataTargetPhase::ErrVerifyFailed => write!(f, "ErrVerifyFailed"),
        }
    }
}
