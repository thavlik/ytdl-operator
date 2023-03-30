use std::{fmt, str::FromStr};

/// Algorithm to use when resizing images. Usually [`Lanczos3`](ImageFilter::Lanczos3) is the best choice.
pub enum ImageFilter {
    /// Basic nearest neighbor sampling strategy. Produces very crude results but is the fastest.
    Nearest,

    /// Basic linear resampling. Produces decent results and is fast.
    Triangle,

    /// Alternative to [bicubic interpolation](https://en.wikipedia.org/wiki/Bicubic_interpolation)
    /// that produces smoother results.
    CatmullRom,

    /// [Gaussian resampling](https://en.wikipedia.org/wiki/Gaussian_filter).
    /// Produces higher quality images, but is slower.
    Gaussian,

    /// [Lanczos multivariate resampling](https://en.wikipedia.org/wiki/Lanczos_resampling).
    /// This option usually yields the highest quality resized images.
    Lanczos3,
}

impl FromStr for ImageFilter {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nearest" => Ok(ImageFilter::Nearest),
            "triangle" => Ok(ImageFilter::Triangle),
            "catmullrom" => Ok(ImageFilter::CatmullRom),
            "gaussian" => Ok(ImageFilter::Gaussian),
            "lanczos3" => Ok(ImageFilter::Lanczos3),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ImageFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageFilter::Nearest => write!(f, "nearest"),
            ImageFilter::Triangle => write!(f, "triangle"),
            ImageFilter::CatmullRom => write!(f, "catmullrom"),
            ImageFilter::Gaussian => write!(f, "gaussian"),
            ImageFilter::Lanczos3 => write!(f, "lanczos3"),
        }
    }
}