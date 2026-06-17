mod exiftool;
pub use exiftool::ExiftoolExtractor;

use async_trait::async_trait;

use crate::error::Result;

/// 抽取并裁剪后的 EXIF（JSON 对象），以及派生出的 ISO 拍摄时间与 orientation。
pub struct ExifResult {
    pub exif: serde_json::Value, // object
    pub date_taken_iso: Option<String>,
    pub orientation: u32,
}

#[async_trait]
pub trait ExifExtractor: Send + Sync {
    /// path：要读取的临时图片文件。
    async fn extract(&self, path: &std::path::Path) -> Result<Option<ExifResult>>;
}

pub const PICK_KEYS: &[&str] = &[
    "tz",
    "tzSource",
    "Orientation",
    "Make",
    "Model",
    "Software",
    "Artist",
    "Copyright",
    "ExposureTime",
    "FNumber",
    "ExposureProgram",
    "ISO",
    "OffsetTime",
    "OffsetTimeOriginal",
    "OffsetTimeDigitized",
    "ShutterSpeedValue",
    "ApertureValue",
    "BrightnessValue",
    "ExposureCompensationSet",
    "ExposureCompensationMode",
    "ExposureCompensationSetting",
    "ExposureCompensation",
    "MaxApertureValue",
    "LightSource",
    "Flash",
    "FocalLength",
    "ColorSpace",
    "ExposureMode",
    "FocalLengthIn35mmFormat",
    "SceneCaptureType",
    "LensMake",
    "LensModel",
    "MeteringMode",
    "WhiteBalance",
    "WBShiftAB",
    "WBShiftGM",
    "WhiteBalanceBias",
    "FlashMeteringMode",
    "SensingMethod",
    "FocalPlaneXResolution",
    "FocalPlaneYResolution",
    "Aperture",
    "ScaleFactor35efl",
    "ShutterSpeed",
    "LightValue",
    "Rating",
    "GPSAltitude",
    "GPSCoordinates",
    "GPSAltitudeRef",
    "GPSLatitude",
    "GPSLatitudeRef",
    "GPSLongitude",
    "GPSLongitudeRef",
    "MPImageType",
    "UniformResourceName",
    "MotionPhoto",
    "MotionPhotoVersion",
    "MotionPhotoPresentationTimestampUs",
    "ContainerDirectory",
    "MicroVideo",
    "MicroVideoVersion",
    "MicroVideoOffset",
    "MicroVideoPresentationTimestampUs",
    "DateTimeOriginal",
    "DateTimeDigitized",
    "ExifImageWidth",
    "ExifImageHeight",
];
