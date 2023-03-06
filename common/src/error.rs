/// All errors possible to occur in the executor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any error originating from the `kube-rs` crate
    #[error("Kubernetes error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },

    /// Any non-credentials errors from `rust-s3` crate
    #[error("S3 service error: {source}")]
    S3Error {
        #[from]
        source: s3::error::S3Error,
    },

    /// Any credentials errors from `rust-s3` crate
    #[error("S3 credentials error: {source}")]
    S3CredentialsError {
        #[from]
        source: awscreds::error::CredentialsError,
    },

    /// Non-200 response from S3
    #[error("S3 upload error code {status_code}")]
    S3UploadError {
        status_code: u16,
    },

    /// Error converting a string to UTF-8
    #[error("UTF-8 error: {source}")]
    Utf8Error {
        #[from]
        source: std::str::Utf8Error,
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

    #[error("VPN sidecar failure: {0}")]
    VPNSidecarFailure(String),

    #[error("system time error: {source}")]
    SystemTimeError {
        #[from]
        source: std::time::SystemTimeError,
    },

    /// Error in user input or Executor resource definition, typically missing fields.
    #[error("Invalid Executor CRD: {0}")]
    UserInputError(String),

    /// Executor status.phase value does not match any known phase.
    #[error("Invalid Executor status.phase: {0}")]
    InvalidPhase(String),

    /// Generic error based on a string description
    #[error("error: {0}")]
    GenericError(String),

    #[error("youtube-dl exit code {exit_code}")]
    YoutubeDlError {
        exit_code: i32,
    },

    #[error("thumbnail download error: {status_code}")]
    ThumbnailDownloadError {
        status_code: u16,
    },

    #[error("reqwest error: {source}")]
    ReqwestError {
        #[from]
        source: reqwest::Error,
    },
}

/*
/// All errors possible to occur during reconciliation
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any error originating from the `kube-rs` crate
    #[error("Kubernetes error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },

    /// Any non-credentials errors from `rust-s3` crate
    #[error("S3 service error: {source}")]
    S3Error {
        #[from]
        source: s3::error::S3Error,
    },

    /// Any credentials errors from `rust-s3` crate
    #[error("S3 credentials error: {source}")]
    S3CredentialsError {
        #[from]
        source: awscreds::error::CredentialsError,
    },

    

    /// Error converting a string to UTF-8
    #[error("UTF-8 error: {source}")]
    Utf8Error {
        #[from]
        source: std::str::Utf8Error,
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
}
*/