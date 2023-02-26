use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Struct corresponding to the Specification (`spec`) part of the `Echo` resource, directly
/// reflects context of the `echoes.example.com.yaml` file to be found in this repository.
/// The `Echo` struct will be generated by the `CustomResource` derive macro.
#[derive(CustomResource, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "example.com",
    version = "v1",
    kind = "Echo",
    plural = "echoes",
    derive = "PartialEq",
    namespaced
)]
pub struct EchoSpec {
    pub replicas: i32,
}