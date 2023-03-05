/// All errors possible to occur in the executor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any error originating from the `kube-rs` crate
    #[error("Kubernetes error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },

    /// Serde json decode error
    #[error("decode json error: {source}")]
    JSONError {
        #[from]
        source: serde_json::Error,
    },

    /// Environment variable error
    #[error("missing environment variable: {source}")]
    EnvError {
        #[from]
        source: std::env::VarError,
    },

    #[error("i/o error: {source}")]
    IOError {
        #[from]
        source: std::io::Error,
    },

    #[error("VPN ready file not found before deadline")]
    ReadyFileNotFound,

    #[error("system time error: {source}")]
    SystemTimeError {
        #[from]
        source: std::time::SystemTimeError,
    },
}