use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::*;

/// Configuration for Redis metadata output. The executor will connect to
/// the Redis cluster and cache the relevant payload at the specified key.
/// Arbitrary scripts are also supported by setting the `script` field.
#[derive(CustomResource, Serialize, Default, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
    group = "ytdl.beebs.dev",
    version = "v1",
    kind = "RedisTarget",
    plural = "redistargets",
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
pub struct RedisTargetSpec {
    /// Name of the Kubernetes [`Secret`](https://kubernetes.io/docs/concepts/configuration/secret/)
    /// resource containing the database credentials. The secret must contain
    /// the following fields:
    ///     - `username`
    ///     - `password`
    ///     - `host`
    ///     - `port`
    ///     - `database`
    ///     - `sslmode`
    ///     - `sslcert` (where necessary)
    pub secret: String,

    /// Template for the redis key. Refer to the youtube-dl documentation on output templates:
    /// <https://github.com/ytdl-org/youtube-dl/blob/master/README.md#output-template>
    /// Default is `"%(id)s.%(ext)s"`. You should consider if prefixing your keys with a
    /// namespace to prevent collisions with other keys in the database is necessary.
    pub key: Option<String>,

    /// Optional script to run instead of the default `SET` command. The script
    /// consumes the following arguments:
    ///   - `KEYS[1]`: the key to set, created from the above `key` template
    ///   - `ARGV[1]`: the AV file, metadata json, or thumbnail file
    ///   - `ARGV[2]`: the metadata json (if ARGV[1] is a video or thumbnail, otherwise unset)
    /// The default script would thus be:
    /// ```lua
    /// redis.call("SET", KEYS[1], ARGV[1])
    /// ```
    /// You should use the [built-in Redis `cjson` library](https://redis.io/docs/manual/programmability/lua-api/#cjson-library)
    /// to deserialize the metadata json in your scripts (available since Redis 2.6.0).
    /// Along with the `extraKeys` field, you are able to arbitrarily manipulate
    /// the redis cluster in response to a successful video download.
    pub script: Option<String>,

    /// Extra keys to pass to the script. These are templated in the same way
    /// as the `key` field. The first key in this list will be indexed starting
    /// at `KEYS[2]` within the script.
    #[serde(rename = "extraKeys")]
    pub extra_keys: Option<Vec<String>>,

    /// Verification settings for the Redis service. The credentials are verified
    /// by dialing the server and executing a ping command. Default behavior is to
    /// verify the credentials once and never again.
    pub verify: Option<TargetVerifySpec>,
}
