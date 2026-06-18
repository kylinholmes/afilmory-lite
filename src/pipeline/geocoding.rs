//! 反查地理编码：从 EXIF 的 GPS 字段解析坐标，调用 Mapbox / Nominatim 得到城市/国家。
//!
//! - 数据源：`auto`（有 mapbox_token 用 Mapbox，否则 Nominatim）/ `mapbox` / `nominatim`。
//! - 限速：每 provider 串行 + 最小间隔（Mapbox 100ms，Nominatim 1s，遵守其 ToS）。
//! - 缓存：单次构建内存缓存，按坐标四舍五入到 `cache_precision` 位去重（含失败结果）。
//! - 坐标始终记入 `LocationInfo`，城市/国家是尽力而为（网络失败时仅保留经纬度）。

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::{GeoProvider, GeocodingConfig};
use crate::manifest::LocationInfo;

enum Resolved {
    Mapbox { token: String },
    Nominatim { base_url: String },
}

pub struct Geocoder {
    resolved: Option<Resolved>, // None = 关闭（或 provider=mapbox 但缺 token）
    client: reqwest::Client,
    language: Option<String>,
    cache_precision: usize,
    min_interval: Duration,
    cache: Mutex<HashMap<String, Option<LocationInfo>>>,
    last_call: Mutex<Option<Instant>>,
}

impl Geocoder {
    pub fn new(cfg: &GeocodingConfig) -> Self {
        let token = cfg
            .mapbox_token
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string);
        let resolved = if !cfg.enabled {
            None
        } else {
            match cfg.provider {
                GeoProvider::Nominatim => Some(Resolved::Nominatim {
                    base_url: cfg.nominatim_base_url.clone(),
                }),
                GeoProvider::Mapbox => match token {
                    Some(t) => Some(Resolved::Mapbox { token: t }),
                    None => {
                        tracing::warn!(
                            "geocoding provider=mapbox 但未配置 mapbox_token，已禁用地理编码"
                        );
                        None
                    }
                },
                GeoProvider::Auto => match token {
                    Some(t) => Some(Resolved::Mapbox { token: t }),
                    None => Some(Resolved::Nominatim {
                        base_url: cfg.nominatim_base_url.clone(),
                    }),
                },
            }
        };
        let min_interval = match &resolved {
            Some(Resolved::Mapbox { .. }) => Duration::from_millis(100),
            _ => Duration::from_millis(1000),
        };
        let client = reqwest::Client::builder()
            .user_agent("afilmory-lite/0.1 (+https://github.com/Afilmory/afilmory)")
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            resolved,
            client,
            language: cfg
                .language
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string),
            cache_precision: cfg.cache_precision,
            min_interval,
            cache: Mutex::new(HashMap::new()),
            last_call: Mutex::new(None),
        }
    }

    pub fn enabled(&self) -> bool {
        self.resolved.is_some()
    }

    /// 从 EXIF 解析 GPS 并反查。未启用 / 无有效坐标 / 解析失败 → None。
    pub async fn locate(&self, exif: Option<&Value>) -> Option<LocationInfo> {
        self.resolved.as_ref()?;
        let (lat, lon) = parse_gps(exif?)?;
        let key = format!(
            "{:.*},{:.*}",
            self.cache_precision, lat, self.cache_precision, lon
        );
        if let Some(hit) = self.cache.lock().await.get(&key).cloned() {
            return hit;
        }
        let result = Some(self.reverse(lat, lon).await);
        self.cache.lock().await.insert(key, result.clone());
        result
    }

    async fn reverse(&self, lat: f64, lon: f64) -> LocationInfo {
        let base = LocationInfo {
            latitude: lat,
            longitude: lon,
            country: None,
            city: None,
            location_name: None,
        };
        let Some(resolved) = &self.resolved else {
            return base;
        };
        self.throttle().await;
        let enriched = match resolved {
            Resolved::Mapbox { token } => self
                .fetch_mapbox(lat, lon, token)
                .await
                .and_then(|v| parse_mapbox(&v, lat, lon)),
            Resolved::Nominatim { base_url } => self
                .fetch_nominatim(lat, lon, base_url)
                .await
                .and_then(|v| parse_nominatim(&v, lat, lon)),
        };
        match enriched {
            Some(loc) => loc,
            None => {
                tracing::warn!("reverse geocode failed for ({lat}, {lon}); keeping coords only");
                base
            }
        }
    }

    /// 串行限速：持锁跨越 sleep，保证每 provider 不超过 1/min_interval 的速率。
    async fn throttle(&self) {
        let mut last = self.last_call.lock().await;
        if let Some(t) = *last {
            let elapsed = t.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn fetch_nominatim(&self, lat: f64, lon: f64, base_url: &str) -> Option<Value> {
        let url = format!("{}/reverse", base_url.trim_end_matches('/'));
        let mut req = self.client.get(&url).query(&[
            ("lat", lat.to_string()),
            ("lon", lon.to_string()),
            ("format", "json".to_string()),
            ("zoom", "10".to_string()),
        ]);
        if let Some(lang) = &self.language {
            req = req.header("accept-language", lang);
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.bytes().await.ok()?;
        serde_json::from_slice(&body).ok()
    }

    async fn fetch_mapbox(&self, lat: f64, lon: f64, token: &str) -> Option<Value> {
        let mut query = vec![
            ("longitude", lon.to_string()),
            ("latitude", lat.to_string()),
            ("access_token", token.to_string()),
        ];
        if let Some(lang) = &self.language {
            query.push(("language", lang.clone()));
        }
        let resp = self
            .client
            .get("https://api.mapbox.com/search/geocode/v6/reverse")
            .query(&query)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.bytes().await.ok()?;
        serde_json::from_slice(&body).ok()
    }
}

static NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+(?:\.[0-9]+)?").unwrap());

/// 从 EXIF JSON 取 GPSLatitude/GPSLongitude（+Ref）并解析为十进制经纬度。
pub fn parse_gps(exif: &Value) -> Option<(f64, f64)> {
    let lat = parse_coord(exif.get("GPSLatitude")?, exif.get("GPSLatitudeRef"))?;
    let lon = parse_coord(exif.get("GPSLongitude")?, exif.get("GPSLongitudeRef"))?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    if lat == 0.0 && lon == 0.0 {
        return None; // Null Island：几乎肯定是无效/缺失坐标
    }
    Some((lat, lon))
}

/// 解析单个坐标：兼容纯数字、带符号小数字符串、exiftool DMS（`37 deg 48' 30.00" N`）。
/// 半球由值尾字母或 Ref 字段（North/South/East/West 或 N/S/E/W）决定。
fn parse_coord(value: &Value, ref_field: Option<&Value>) -> Option<f64> {
    let (mag, hemi_from_val) = magnitude_and_hemi(value)?;
    let hemi = hemi_from_val.or_else(|| ref_field.and_then(hemisphere_of));
    let signed = match hemi {
        Some('S') | Some('W') => -mag.abs(),
        Some('N') | Some('E') => mag.abs(),
        _ => mag, // 无半球信息 → 保留解析出的符号（带符号小数 / 数字）
    };
    Some(signed)
}

fn magnitude_and_hemi(value: &Value) -> Option<(f64, Option<char>)> {
    if let Some(n) = value.as_f64() {
        return Some((n, None));
    }
    let s = value.as_str()?.trim();
    let mut hemi = None;
    let mut body = s;
    if let Some(last) = s.chars().last() {
        let u = last.to_ascii_uppercase();
        if matches!(u, 'N' | 'S' | 'E' | 'W') {
            hemi = Some(u);
            body = s[..s.len() - last.len_utf8()].trim_end();
        }
    }
    let is_dms = body.contains("deg") || body.contains('\'') || body.contains('"');
    let mag = if is_dms {
        let mut nums = NUM_RE.find_iter(body).filter_map(|m| m.as_str().parse::<f64>().ok());
        let deg = nums.next()?;
        let min = nums.next().unwrap_or(0.0);
        let sec = nums.next().unwrap_or(0.0);
        deg + min / 60.0 + sec / 3600.0
    } else {
        body.parse::<f64>().ok()?
    };
    Some((mag, hemi))
}

fn hemisphere_of(v: &Value) -> Option<char> {
    let s = v.as_str()?.trim();
    let c = s.chars().next()?.to_ascii_uppercase();
    matches!(c, 'N' | 'S' | 'E' | 'W').then_some(c)
}

/// 取按优先级排列、去重后的前 2 个非空名称，逗号连接。
fn join_top2<'a>(parts: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for p in parts.flatten() {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
            if out.len() == 2 {
                break;
            }
        }
    }
    (!out.is_empty()).then(|| out.join(", "))
}

fn parse_nominatim(v: &Value, lat: f64, lon: f64) -> Option<LocationInfo> {
    let addr = v.get("address")?;
    let get = |k: &str| addr.get(k).and_then(|x| x.as_str());
    let country = get("country")
        .map(str::to_string)
        .or_else(|| get("country_code").map(|c| c.to_uppercase()));
    let city = join_top2(
        [
            "village",
            "hamlet",
            "neighbourhood",
            "suburb",
            "district",
            "city",
            "town",
            "county",
            "state",
        ]
        .into_iter()
        .map(get),
    );
    let location_name = v.get("display_name").and_then(|x| x.as_str()).map(str::to_string);
    Some(LocationInfo {
        latitude: lat,
        longitude: lon,
        country,
        city,
        location_name,
    })
}

fn parse_mapbox(v: &Value, lat: f64, lon: f64) -> Option<LocationInfo> {
    let props = v.get("features")?.as_array()?.first()?.get("properties")?;
    let ctx = props.get("context")?;
    let name_of = |k: &str| ctx.get(k).and_then(|c| c.get("name")).and_then(|x| x.as_str());
    let country = name_of("country").map(str::to_string);
    let city = join_top2(
        ["locality", "neighborhood", "district", "place", "region"]
            .into_iter()
            .map(name_of),
    );
    let location_name = props
        .get("place_formatted")
        .and_then(|x| x.as_str())
        .or_else(|| props.get("name").and_then(|x| x.as_str()))
        .map(str::to_string);
    Some(LocationInfo {
        latitude: lat,
        longitude: lon,
        country,
        city,
        location_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn gps_dms_with_trailing_hemisphere() {
        // Composite 风格：值尾带半球字母
        let exif = json!({
            "GPSLatitude": "37 deg 48' 30.00\" N",
            "GPSLongitude": "122 deg 25' 12.00\" W",
        });
        let (lat, lon) = parse_gps(&exif).unwrap();
        assert!(approx(lat, 37.8083), "lat={lat}");
        assert!(approx(lon, -122.42), "lon={lon}");
    }

    #[test]
    fn gps_dms_magnitude_plus_ref() {
        // 原始 EXIF 风格：值只有量值，半球在 Ref（全称）
        let exif = json!({
            "GPSLatitude": "37 deg 48' 30.00\"",
            "GPSLatitudeRef": "North",
            "GPSLongitude": "122 deg 25' 12.00\"",
            "GPSLongitudeRef": "West",
        });
        let (lat, lon) = parse_gps(&exif).unwrap();
        assert!(approx(lat, 37.8083));
        assert!(approx(lon, -122.42));
    }

    #[test]
    fn gps_numeric_with_ref() {
        let exif = json!({
            "GPSLatitude": 35.0,
            "GPSLatitudeRef": "S",
            "GPSLongitude": 139.0,
            "GPSLongitudeRef": "E",
        });
        let (lat, lon) = parse_gps(&exif).unwrap();
        assert!(approx(lat, -35.0));
        assert!(approx(lon, 139.0));
    }

    #[test]
    fn gps_signed_decimal_string() {
        let exif = json!({ "GPSLatitude": "-33.8688", "GPSLongitude": "151.2093" });
        let (lat, lon) = parse_gps(&exif).unwrap();
        assert!(approx(lat, -33.8688));
        assert!(approx(lon, 151.2093));
    }

    #[test]
    fn gps_invalid_or_null_island() {
        assert!(parse_gps(&json!({})).is_none());
        assert!(parse_gps(&json!({ "GPSLatitude": 0.0, "GPSLongitude": 0.0 })).is_none());
        assert!(parse_gps(&json!({ "GPSLatitude": 200.0, "GPSLongitude": 10.0 })).is_none());
    }

    #[test]
    fn nominatim_parse() {
        let v = json!({
            "display_name": "Tokyo Tower, Minato, Tokyo, Japan",
            "address": { "city": "Minato", "state": "Tokyo", "country": "Japan", "country_code": "jp" }
        });
        let loc = parse_nominatim(&v, 35.6586, 139.7454).unwrap();
        assert_eq!(loc.country.as_deref(), Some("Japan"));
        assert_eq!(loc.city.as_deref(), Some("Minato, Tokyo"));
        assert_eq!(loc.location_name.as_deref(), Some("Tokyo Tower, Minato, Tokyo, Japan"));
    }

    #[test]
    fn nominatim_country_code_fallback() {
        let v = json!({ "address": { "country_code": "fr", "city": "Paris" } });
        let loc = parse_nominatim(&v, 48.85, 2.35).unwrap();
        assert_eq!(loc.country.as_deref(), Some("FR"));
        assert_eq!(loc.city.as_deref(), Some("Paris"));
    }

    #[test]
    fn mapbox_parse() {
        let v = json!({
            "features": [{
                "properties": {
                    "name": "San Francisco",
                    "place_formatted": "San Francisco, California, United States",
                    "context": {
                        "country": { "name": "United States" },
                        "place": { "name": "San Francisco" },
                        "region": { "name": "California" }
                    }
                }
            }]
        });
        let loc = parse_mapbox(&v, 37.77, -122.42).unwrap();
        assert_eq!(loc.country.as_deref(), Some("United States"));
        assert_eq!(loc.city.as_deref(), Some("San Francisco, California"));
        assert_eq!(loc.location_name.as_deref(), Some("San Francisco, California, United States"));
    }

    #[test]
    fn resolution_auto_with_token_is_mapbox_fast() {
        let cfg = GeocodingConfig {
            enabled: true,
            provider: GeoProvider::Auto,
            mapbox_token: Some("pk.abc".into()),
            ..Default::default()
        };
        let g = Geocoder::new(&cfg);
        assert!(g.enabled());
        assert_eq!(g.min_interval, Duration::from_millis(100));
    }

    #[test]
    fn resolution_auto_without_token_is_nominatim_slow() {
        let cfg = GeocodingConfig {
            enabled: true,
            provider: GeoProvider::Auto,
            ..Default::default()
        };
        let g = Geocoder::new(&cfg);
        assert!(g.enabled());
        assert_eq!(g.min_interval, Duration::from_millis(1000));
    }

    #[test]
    fn resolution_mapbox_without_token_disabled() {
        let cfg = GeocodingConfig {
            enabled: true,
            provider: GeoProvider::Mapbox,
            mapbox_token: None,
            ..Default::default()
        };
        assert!(!Geocoder::new(&cfg).enabled());
    }

    #[test]
    fn disabled_when_not_enabled() {
        let g = Geocoder::new(&GeocodingConfig::default());
        assert!(!g.enabled());
    }

    #[tokio::test]
    async fn locate_returns_none_when_disabled() {
        let g = Geocoder::new(&GeocodingConfig::default());
        let exif = json!({ "GPSLatitude": 35.0, "GPSLatitudeRef": "N", "GPSLongitude": 139.0, "GPSLongitudeRef": "E" });
        assert!(g.locate(Some(&exif)).await.is_none());
    }
}
