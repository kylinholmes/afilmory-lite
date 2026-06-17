use chrono::{NaiveDateTime, SecondsFormat, TimeZone, Utc};

pub struct PhotoInfo {
    pub title: String,
    pub date_taken: String,
    pub tags: Vec<String>,
    pub description: String,
}

/// key: 存储 key（含目录）。exif_date_taken: 已格式化为 ISO 的 EXIF 日期（若有）。
pub fn extract_info(key: &str, exif_date_taken: Option<&str>) -> PhotoInfo {
    let key = key.replace('\\', "/");
    let file_name = key.rsplit('/').next().unwrap_or(&key);
    let file_stem = strip_ext(file_name);

    // tags：目录每一级
    let tags: Vec<String> = match key.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() && dir != "." => dir
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect(),
        _ => vec![],
    };

    // dateTaken
    let date_taken = exif_date_taken
        .map(|s| s.to_string())
        .or_else(|| date_from_filename(file_stem))
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));

    // title
    let title = clean_title(file_stem);

    PhotoInfo {
        title,
        date_taken,
        tags,
        description: String::new(),
    }
}

fn strip_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

fn date_from_filename(stem: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap();
    let caps = re.captures(stem)?;
    let s = format!("{}-{}-{}", &caps[1], &caps[2], &caps[3]);
    let ndt = NaiveDateTime::parse_from_str(&format!("{s} 00:00:00"), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(
        Utc.from_utc_datetime(&ndt)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    )
}

fn clean_title(stem: &str) -> String {
    let date_re = regex::Regex::new(r"\d{4}-\d{2}-\d{2}[_-]?").unwrap();
    let views_re = regex::Regex::new(r"(?i)[_-]?\d+views?").unwrap();
    let sep_re = regex::Regex::new(r"[_-]+").unwrap();
    let mut t = date_re.replace_all(stem, "").to_string();
    t = views_re.replace_all(&t, "").to_string();
    t = sep_re.replace_all(&t, " ").to_string();
    let t = t.trim().to_string();
    if t.is_empty() { stem.to_string() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_from_dirs() {
        let i = extract_info("trip/2024/DSC_0001.jpg", None);
        assert_eq!(i.tags, vec!["trip".to_string(), "2024".to_string()]);
    }

    #[test]
    fn date_from_filename_used_when_no_exif() {
        let i = extract_info("2024-05-01_sunset.jpg", None);
        assert_eq!(i.date_taken, "2024-05-01T00:00:00.000Z");
        assert_eq!(i.title, "sunset");
    }

    #[test]
    fn exif_date_takes_priority() {
        let i = extract_info("x/2024-05-01_a.jpg", Some("2024-05-01T10:00:00.000Z"));
        assert_eq!(i.date_taken, "2024-05-01T10:00:00.000Z");
    }
}
