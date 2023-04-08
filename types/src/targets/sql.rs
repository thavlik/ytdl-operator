use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::*;

/// Configuration for SQL-compatible metadata output. Use this if your application
/// is designed to retrieved the metadata json from a SQL database. The executor
/// pods will connect to the database and insert the metadata while querying.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "SqlTarget",
    plural = "sqltargets",
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
pub struct SqlTargetSpec {
    /// Name of the Kubernetes [`Secret`](https://kubernetes.io/docs/concepts/configuration/secret/)
    /// resource containing the SQL database credentials. The secret must contain
    /// the following fields:
    ///     - `username`
    ///     - `password`
    ///     - `host`
    ///     - `port`
    ///     - `database`
    ///     - `sslmode`
    ///     - `sslcert` (where necessary)
    pub secret: String,

    /// Verification settings for the SQL database. Default behavior is to
    /// verify the credentials once and never again.
    pub verify: Option<TargetVerifySpec>,
}
