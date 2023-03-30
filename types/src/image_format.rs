use std::{fmt, str::FromStr};

/// Enumeration of supported image file types for thumbnail conversion.
/// Roughly corresponds with the [`image::ImageFormat`] enum.
pub enum ImageFormat {
    Jpeg,
    Png,
    Webp,
    Bmp,
    Gif,
    Ico,
    Pgm,
}

impl FromStr for ImageFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "jpg" => Ok(ImageFormat::Jpeg),
            "png" => Ok(ImageFormat::Png),
            "webp" => Ok(ImageFormat::Webp),
            "bmp" => Ok(ImageFormat::Bmp),
            "gif" => Ok(ImageFormat::Gif),
            "ico" => Ok(ImageFormat::Ico),
            "pgm" => Ok(ImageFormat::Pgm),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageFormat::Jpeg => write!(f, "jpg"),
            ImageFormat::Png => write!(f, "png"),
            ImageFormat::Webp => write!(f, "webp"),
            ImageFormat::Bmp => write!(f, "bmp"),
            ImageFormat::Gif => write!(f, "gif"),
            ImageFormat::Ico => write!(f, "ico"),
            ImageFormat::Pgm => write!(f, "pgm"),
        }
    }
}