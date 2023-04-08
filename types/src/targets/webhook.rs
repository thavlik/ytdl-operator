use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::common::*;

/// Configuration for a webhook's HTTP Basic Auth.
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema, PartialEq)]
pub struct WebhookBasicAuthSpec {
    /// Name of the Kubernetes [`Secret`](https://kubernetes.io/docs/concepts/configuration/secret/)
    /// resource containing the HTTP basic auth credentials. The secret must contain
    /// the fields `username` and `password`.
    pub secret: String,
}

/// A target resource that makes an HTTP request with the relevant content in the body.
/// Use this to automate processing of the video, such as transcoding, uploading to a CDN,
/// indexing in a search engine, etc.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "WebhookTarget",
    plural = "webhooktargets",
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
pub struct WebhookTargetSpec {
    /// URL template for the webhook. Refer to youtube-dl documentation
    /// for details on which template variables are available:
    /// <https://github.com/ytdl-org/youtube-dl#output-template>.
    /// The body of the request will depend on what is being sent. For
    /// metadata, the request body will be the JSON metadata. For AV
    /// content and thumbnails, the body is the raw file.
    pub url: String,

    /// Verification configuration. Default is to verify the webhook
    /// once and never again. Verification is done by sending a HEAD
    /// request to the webhook URL and checking the response code.
    /// All template variables in the URL are replaced with the string
    /// `"test"`. The server should respond with a 200 status code.
    pub verify: Option<TargetVerifySpec>,

    /// HTTP method override. Default is `"POST"`.
    pub method: Option<String>,

    /// Request timeout duration string. Default is `"10s"`. You will
    /// want to increase this value to something like `"5m"` if you are
    /// sending large AV files.
    pub timeout: Option<String>,

    /// Optional HTTP basic auth configuration.
    #[serde(rename = "basicAuth")]
    pub basic_auth: Option<WebhookBasicAuthSpec>,

    /// Optional map object of HTTP headers to send with the request.
    /// The keys are the header names and the values are the header values.
    /// For HTTP basic auth, it is recommended to use the `basicAuth` field
    /// instead of hard-coding them into this map.
    pub headers: Option<BTreeMap<String, String>>,
}
