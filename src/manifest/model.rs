use serde::{Deserialize, Serialize};

pub const CURRENT_MANIFEST_VERSION: &str = "v10";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfilmoryManifest {
    pub version: String,
    pub data: Vec<PhotoManifestItem>,
    pub cameras: Vec<CameraInfo>,
    pub lenses: Vec<LensInfo>,
}

impl Default for AfilmoryManifest {
    fn default() -> Self {
        Self {
            version: CURRENT_MANIFEST_VERSION.to_string(),
            data: Vec::new(),
            cameras: Vec::new(),
            lenses: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoManifestItem {
    pub id: String,
    pub format: String,
    pub title: String,
    pub description: String,
    pub date_taken: String,
    pub tags: Vec<String>,
    pub original_url: String,
    pub thumbnail_url: String,
    // 必出可为 null：
    pub thumb_hash: Option<String>,
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: f64,
    pub s3_key: String,
    pub last_modified: String,
    pub size: u64,
    // 无值省略键：
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    // 必出可为 null：
    pub exif: Option<serde_json::Value>,
    pub tone_analysis: Option<ToneAnalysis>,
    pub location: Option<LocationInfo>,
    // 无值省略键：
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoSource>,
    #[serde(rename = "isHDR")]
    pub is_hdr: bool,
    #[serde(rename = "ogImageUrl", skip_serializing_if = "Option::is_none")]
    pub og_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneAnalysis {
    pub tone_type: String, // 'low-key' | 'high-key' | 'normal' | 'high-contrast'
    pub brightness: u32,
    pub contrast: u32,
    pub shadow_ratio: f64,
    pub highlight_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationInfo {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VideoSource {
    #[serde(rename = "live-photo")]
    LivePhoto {
        #[serde(rename = "videoUrl")]
        video_url: String,
        #[serde(rename = "s3Key")]
        s3_key: String,
    },
    #[serde(rename = "motion-photo")]
    MotionPhoto {
        offset: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        #[serde(rename = "presentationTimestamp", skip_serializing_if = "Option::is_none")]
        presentation_timestamp: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub make: String,
    pub model: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    pub model: String,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> PhotoManifestItem {
        PhotoManifestItem {
            id: "DSC_0001".into(),
            format: "JPG".into(),
            title: "DSC 0001".into(),
            description: String::new(),
            date_taken: "2024-01-01T12:00:00.000Z".into(),
            tags: vec!["trip".into()],
            original_url: "https://cdn/DSC_0001.jpg".into(),
            thumbnail_url: "/thumbnails/DSC_0001.jpg".into(),
            thumb_hash: None,
            width: 4000,
            height: 3000,
            aspect_ratio: 4000.0 / 3000.0,
            s3_key: "trip/DSC_0001.jpg".into(),
            last_modified: "2024-01-01T12:00:00.000Z".into(),
            size: 123,
            digest: None,
            exif: None,
            tone_analysis: None,
            location: None,
            video: None,
            is_hdr: false,
            og_image_url: None,
        }
    }

    #[test]
    fn null_fields_present_optional_fields_omitted() {
        let json = serde_json::to_value(sample_item()).unwrap();
        let obj = json.as_object().unwrap();
        // 必出可 null
        assert!(obj.contains_key("thumbHash"));
        assert!(obj["thumbHash"].is_null());
        assert!(obj.contains_key("exif") && obj["exif"].is_null());
        assert!(obj.contains_key("toneAnalysis") && obj["toneAnalysis"].is_null());
        assert!(obj.contains_key("location") && obj["location"].is_null());
        // 无值省略键
        assert!(!obj.contains_key("digest"));
        assert!(!obj.contains_key("video"));
        assert!(!obj.contains_key("ogImageUrl"));
        // 恒出
        assert_eq!(obj["isHDR"], serde_json::json!(false));
        // 不规则键名
        assert!(obj.contains_key("s3Key"));
        assert!(obj.contains_key("dateTaken"));
    }

    #[test]
    fn video_source_tagged() {
        let v = VideoSource::LivePhoto { video_url: "u".into(), s3_key: "k".into() };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["type"], "live-photo");
        assert_eq!(j["videoUrl"], "u");
        assert_eq!(j["s3Key"], "k");
    }
}
