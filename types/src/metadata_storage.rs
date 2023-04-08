use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

use crate::common::*;

/// Struct corresponding to the Specification (`spec`) part of the `MetadataStorage` resource.
/// The `MetadataStorage` custom resource is responsible for configuring the storage of metadata.
#[derive(CustomResource, Default, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "MetadataStorage",
    plural = "metadatastorages",
    status = "MetadataStorageStatus",
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
pub enum MetadataStorageSpec {
    /// Amazon S3-compatible output for metadata `.json` files. Each video's metadata
    /// will be stored in a separate `.json` file in the specified bucket.
    S3(S3OutputSpec),

    /// SQL-compatible output for the metadata. The executor uses the
    /// [`sqlx`](https://crates.io/crates/sqlx) crate to connect to the database,
    /// which supports a wide variety of SQL-compatible databases.
    Sql(SqlOutputSpec),

    /// MongoDB-compatible output for the metadata. The executor uses the
    /// [`mongodb`](https://crates.io/crates/mongodb) crate to connect to the database
    /// and store the metadata json as-is in a single collection.
    MongoDB(MongoDBOutputSpec),

    /// Redis cache configuration for the metadata. The json is stored as a key
    /// in the specified Redis cluster for each video.
    Redis(RedisOutputSpec),
}

/// Status object for the [`MetadataStorage`] resource.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct MetadataStorageStatus {
    /// A short description of the [`MetadataStorage`] resource's current state.
    pub phase: Option<MetadataStoragePhase>,

    /// A human-readable message indicating details about why the
    /// [`MetadataStorage`] is in this phase.
    pub message: Option<String>,

    /// Timestamp of when the [`MetadataStorageStatus`] object was last updated.
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,

    /// Timestamp of when all of the service credentials were last verified.
    #[serde(rename = "lastVerified")]
    pub last_verified: Option<String>,
}

/// A short description of the [`MetadataStorage`] resource's current state.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
pub enum MetadataStoragePhase {
    /// The [`MetadataStorage`] resource first appeared to the controller.
    Pending,

    /// The controller is testing the storage backend service with the
    /// credentials to ensure they are valid.
    Verifying,

    /// The storage backend is ready to be used.
    Ready,

    /// The credentials test(s) failed with an error.
    ErrVerifyFailed,
}

impl FromStr for MetadataStoragePhase {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(MetadataStoragePhase::Pending),
            "Verifying" => Ok(MetadataStoragePhase::Verifying),
            "Ready" => Ok(MetadataStoragePhase::Ready),
            "ErrVerifyFailed" => Ok(MetadataStoragePhase::ErrVerifyFailed),
            _ => Err(()),
        }
    }
}

impl fmt::Display for MetadataStoragePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetadataStoragePhase::Pending => write!(f, "Pending"),
            MetadataStoragePhase::Verifying => write!(f, "Verifying"),
            MetadataStoragePhase::Ready => write!(f, "Ready"),
            MetadataStoragePhase::ErrVerifyFailed => write!(f, "ErrVerifyFailed"),
        }
    }
}
