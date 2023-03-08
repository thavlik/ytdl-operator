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
    S3UploadError { status_code: u16 },

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

    /// Compatibility with std
    #[error("i/o error: {source}")]
    IOError {
        #[from]
        source: std::io::Error,
    },

    /// Issue waiting for the VPN to connect.
    #[error("VPN error: {0}")]
    VPNError(String),

    /// Error querying system time.
    #[error("system time error: {source}")]
    SystemTimeError {
        #[from]
        source: std::time::SystemTimeError,
    },

    /// Error in user input or Executor resource definition, typically missing fields.
    /// Prefer this over UnknownError whenever it makes sense.
    #[error("Invalid user input: {0}")]
    UserInputError(String),

    /// Executor status.phase value does not match any known phase.
    #[error("Invalid Executor status.phase: {0}")]
    InvalidPhase(String),

    /// Generic error based on a string description. Try to minimize use of this.
    #[error("uncategorized error: {0}")]
    UnknownError(String),

    /// Nonzero exit code from youtube-dl.
    #[error("youtube-dl exit code {exit_code}")]
    YoutubeDlError { exit_code: i32 },

    /// Non-200 response when downloading thumbnail.
    #[error("thumbnail download error: {status_code}")]
    ThumbnailDownloadError { status_code: u16 },

    /// Generic HTTP client error.
    #[error("reqwest http client error: {source}")]
    ReqwestError {
        #[from]
        source: reqwest::Error,
    },

    /// Image error, e.g. corrupt jpg file.
    #[error("image error: {source}")]
    ImageError {
        #[from]
        source: image::error::ImageError,
    },

    #[error("pod scheduling error: {0}")]
    PodSchedulingError(String),
}
