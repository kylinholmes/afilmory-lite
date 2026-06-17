//! S3 及 S3 兼容存储（AWS / MinIO / Cloudflare R2 / Wasabi 等）。
//! 手写 SigV4 + reqwest；URL 拼接与上游对齐（见 inventory §3/§6）。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::error::{Error, Result};
use crate::storage::sigv4;
use crate::storage::{StorageObject, StorageProvider, is_image_key};

// ---------------- URL / key 编码 ----------------

/// JS `encodeURIComponent` 等价：保留 `A-Za-z0-9-_.!~*'()`，其余百分号编码。用于公开 URL。
fn enc_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')')
        {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// RFC3986 编码：仅保留 `A-Za-z0-9-_.~`。用于 SigV4 canonical URI / query。
fn enc_rfc3986(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// 公开 URL 的 key 编码：逐段 encodeURIComponent，保留 `/`。
pub fn encode_s3_key(key: &str) -> String {
    key.split('/')
        .map(enc_uri_component)
        .collect::<Vec<_>>()
        .join("/")
}

/// 把 key 逐段 RFC3986 编码（用于请求/签名路径），保留 `/`。
fn rfc3986_key(key: &str) -> String {
    key.split('/')
        .map(enc_rfc3986)
        .collect::<Vec<_>>()
        .join("/")
}

/// base URL（结尾恒带 `/`），用于公开 URL 生成。
pub fn build_base_url(bucket: &str, region: &str, endpoint: Option<&str>) -> String {
    match endpoint {
        None => format!("https://{bucket}.s3.{region}.amazonaws.com/"),
        Some(ep) => {
            let trimmed = ep.trim_end_matches('/');
            if trimmed.contains("{bucket}") {
                format!("{}/", trimmed.replace("{bucket}", bucket))
            } else {
                format!("{trimmed}/{bucket}/")
            }
        }
    }
}

/// 生成公开 URL：custom_domain 优先（**不 encode** key）；否则 base_url + 编码 key。
pub fn generate_public_url(key: &str, base_url: &str, custom_domain: Option<&str>) -> String {
    match custom_domain {
        Some(cd) if !cd.is_empty() => format!("{}/{}", cd.trim_end_matches('/'), key),
        _ => format!("{base_url}{}", encode_s3_key(key)),
    }
}

/// 规范化 query：逐项 RFC3986 编码 key/value，按 key 排序，`k=v&..` 拼接。
fn canonical_query(params: &[(String, String)]) -> String {
    let mut p: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (enc_rfc3986(k), enc_rfc3986(v)))
        .collect();
    p.sort();
    p.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

// ---------------- 分页阈值（与上游一致） ----------------

/// max_file_limit ≤ 1000 时只发一页（不翻页）。
fn should_paginate(max_total: Option<usize>) -> bool {
    max_total.is_none_or(|m| m > 1000)
}

/// 每页 max-keys：min(剩余, 1000)；无限制则 1000。
fn page_size(max_total: Option<usize>, already: usize) -> u32 {
    match max_total {
        Some(m) => {
            let remaining = m.saturating_sub(already);
            remaining.clamp(1, 1000) as u32
        }
        None => 1000,
    }
}

// ---------------- ListObjectsV2 XML ----------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListBucketResult {
    #[serde(default)]
    contents: Vec<Contents>,
    #[serde(default)]
    is_truncated: bool,
    #[serde(default)]
    next_continuation_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Contents {
    key: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(rename = "ETag", default)]
    etag: Option<String>,
}

fn parse_s3_date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn to_object(c: Contents) -> StorageObject {
    StorageObject {
        key: c.key,
        size: Some(c.size),
        last_modified: c.last_modified.as_deref().and_then(parse_s3_date),
        etag: c.etag.map(|e| e.trim_matches('"').to_string()),
    }
}

fn parse_list_xml(xml: &str) -> Result<ListBucketResult> {
    quick_xml::de::from_str(xml).map_err(|e| Error::Storage(format!("s3 list parse: {e}")))
}

// ---------------- S3Provider ----------------

pub struct S3Provider {
    client: reqwest::Client,
    region: String,
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    prefix: Option<String>,
    custom_domain: Option<String>,
    exclude: Option<regex::Regex>,
    max_file_limit: Option<usize>,
    sem: Arc<Semaphore>,
    base_url: String,
    // 内部请求用：scheme/host/path_prefix（virtual-host 时 path_prefix 为空，path-style 时为 "/bucket"）
    scheme: String,
    host: String,
    path_prefix: String,
}

#[allow(clippy::too_many_arguments)]
impl S3Provider {
    pub fn new(
        bucket: String,
        region: String,
        endpoint: Option<String>,
        access_key: String,
        secret_key: String,
        session_token: Option<String>,
        prefix: Option<String>,
        custom_domain: Option<String>,
        exclude_regex: Option<String>,
        max_file_limit: Option<usize>,
        download_concurrency: usize,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| Error::Storage(format!("build http client: {e}")))?;
        let (scheme, host, path_prefix) = resolve_endpoint(&bucket, &region, endpoint.as_deref())?;
        let base_url = build_base_url(&bucket, &region, endpoint.as_deref());
        let exclude = match exclude_regex {
            Some(r) => Some(
                regex::Regex::new(&r)
                    .map_err(|e| Error::Config(format!("bad exclude_regex: {e}")))?,
            ),
            None => None,
        };
        Ok(Self {
            client,
            region,
            access_key,
            secret_key,
            session_token,
            prefix,
            custom_domain,
            exclude,
            max_file_limit,
            sem: Arc::new(Semaphore::new(download_concurrency.max(1))),
            base_url,
            scheme,
            host,
            path_prefix,
        })
    }

    pub fn public_url(&self, key: &str) -> String {
        generate_public_url(key, &self.base_url, self.custom_domain.as_deref())
    }

    /// 发起一个已签名的 GET 请求。`encoded_path` 为请求路径（已 RFC3986 编码、含 path_prefix）。
    async fn send_get(
        &self,
        encoded_path: &str,
        canonical_query: &str,
    ) -> Result<reqwest::Response> {
        let amz_date = current_amz_date();
        let payload = sigv4::EMPTY_PAYLOAD_SHA256;

        let mut sign_headers = vec![
            ("host".to_string(), self.host.clone()),
            ("x-amz-content-sha256".to_string(), payload.to_string()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(tok) = &self.session_token {
            sign_headers.push(("x-amz-security-token".to_string(), tok.clone()));
        }

        let signed = sigv4::sign(&sigv4::SigningParams {
            method: "GET",
            canonical_uri: encoded_path,
            canonical_query,
            headers: &sign_headers,
            payload_sha256_hex: payload,
            region: &self.region,
            service: "s3",
            access_key: &self.access_key,
            secret_key: &self.secret_key,
            amz_date: &amz_date,
        });

        let url = if canonical_query.is_empty() {
            format!("{}://{}{}", self.scheme, self.host, encoded_path)
        } else {
            format!(
                "{}://{}{}?{}",
                self.scheme, self.host, encoded_path, canonical_query
            )
        };

        let mut req = self
            .client
            .get(&url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", payload)
            .header("authorization", signed.authorization);
        if let Some(tok) = &self.session_token {
            req = req.header("x-amz-security-token", tok);
        }
        req.send()
            .await
            .map_err(|e| Error::Storage(format!("s3 request failed: {e}")))
    }

    fn list_path(&self) -> String {
        if self.path_prefix.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", self.path_prefix)
        }
    }

    /// 列出（受 prefix / max_file_limit 限制的）全部对象。
    pub async fn list_objects(&self) -> Result<Vec<StorageObject>> {
        let max_total = self.max_file_limit;
        let paginate = should_paginate(max_total);
        let path = self.list_path();
        let mut all: Vec<StorageObject> = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let mut params: Vec<(String, String)> = vec![
                ("list-type".into(), "2".into()),
                (
                    "max-keys".into(),
                    page_size(max_total, all.len()).to_string(),
                ),
            ];
            if let Some(p) = &self.prefix
                && !p.is_empty()
            {
                params.push(("prefix".into(), p.clone()));
            }
            if let Some(t) = &token {
                params.push(("continuation-token".into(), t.clone()));
            }
            let cq = canonical_query(&params);

            let resp = self.send_get(&path, &cq).await?;
            if !resp.status().is_success() {
                let code = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(Error::Storage(format!("s3 list failed: {code} {body}")));
            }
            let xml = resp
                .text()
                .await
                .map_err(|e| Error::Storage(e.to_string()))?;
            let parsed = parse_list_xml(&xml)?;

            for c in parsed.contents {
                if c.key.is_empty() {
                    continue;
                }
                all.push(to_object(c));
            }
            if let Some(m) = max_total
                && all.len() >= m
            {
                break;
            }
            if !paginate || !parsed.is_truncated {
                break;
            }
            match parsed.next_continuation_token {
                Some(t) if !t.is_empty() => token = Some(t),
                _ => break,
            }
        }

        if let Some(m) = max_total {
            all.truncate(m);
        }
        Ok(all)
    }

    /// 下载对象字节；不存在(404)或重试耗尽返回 None（不抛，与上游一致）。
    pub async fn get_object(&self, key: &str) -> Result<Option<Bytes>> {
        let _permit = self.sem.acquire().await.expect("semaphore closed");
        let encoded_path = format!("{}/{}", self.path_prefix, rfc3986_key(key));
        const MAX_ATTEMPTS: u32 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            match self.send_get(&encoded_path, "").await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.bytes().await {
                            Ok(b) => return Ok(Some(b)),
                            Err(e) => {
                                tracing::warn!("s3 get body error {key} (try {attempt}): {e}")
                            }
                        }
                    } else if status.as_u16() == 404 {
                        return Ok(None);
                    } else {
                        tracing::warn!("s3 get {key} status {status} (try {attempt})");
                    }
                }
                Err(e) => tracing::warn!("s3 get {key} error (try {attempt}): {e}"),
            }
            if attempt < MAX_ATTEMPTS {
                let backoff = (300u64 * (1u64 << (attempt - 1))).min(4000);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
        }
        Ok(None)
    }
}

fn current_amz_date() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

fn resolve_endpoint(
    bucket: &str,
    region: &str,
    endpoint: Option<&str>,
) -> Result<(String, String, String)> {
    match endpoint {
        None => Ok((
            "https".into(),
            format!("{bucket}.s3.{region}.amazonaws.com"),
            String::new(),
        )),
        Some(ep) => {
            let trimmed = ep.trim_end_matches('/');
            if trimmed.contains("{bucket}") {
                let u = reqwest::Url::parse(&trimmed.replace("{bucket}", bucket))
                    .map_err(|e| Error::Config(format!("bad endpoint: {e}")))?;
                Ok((u.scheme().into(), host_port(&u)?, String::new()))
            } else {
                let u = reqwest::Url::parse(trimmed)
                    .map_err(|e| Error::Config(format!("bad endpoint: {e}")))?;
                Ok((u.scheme().into(), host_port(&u)?, format!("/{bucket}")))
            }
        }
    }
}

fn host_port(u: &reqwest::Url) -> Result<String> {
    let h = u
        .host_str()
        .ok_or_else(|| Error::Config("endpoint missing host".into()))?;
    Ok(match u.port() {
        Some(p) => format!("{h}:{p}"),
        None => h.to_string(),
    })
}

#[async_trait]
impl StorageProvider for S3Provider {
    async fn list_images(&self) -> Result<Vec<StorageObject>> {
        let all = self.list_objects().await?;
        Ok(all
            .into_iter()
            .filter(|o| is_image_key(&o.key))
            .filter(|o| self.exclude.as_ref().is_none_or(|re| !re.is_match(&o.key)))
            .collect())
    }

    async fn list_all_files(&self) -> Result<Vec<StorageObject>> {
        self.list_objects().await
    }

    async fn get_file(&self, key: &str) -> Result<Option<Bytes>> {
        self.get_object(key).await
    }

    fn generate_public_url(&self, key: &str) -> String {
        self.public_url(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_key_per_segment_preserving_slash() {
        assert_eq!(
            encode_s3_key("trip/2024 a/照片.jpg"),
            "trip/2024%20a/%E7%85%A7%E7%89%87.jpg"
        );
        assert_eq!(encode_s3_key("a-b_c.d/e(f)"), "a-b_c.d/e(f)");
    }

    #[test]
    fn rfc3986_encodes_more() {
        assert_eq!(rfc3986_key("e(f)/g h"), "e%28f%29/g%20h");
    }

    #[test]
    fn base_url_branches() {
        assert_eq!(
            build_base_url("b", "us-east-1", None),
            "https://b.s3.us-east-1.amazonaws.com/"
        );
        assert_eq!(
            build_base_url("b", "x", Some("https://minio.local")),
            "https://minio.local/b/"
        );
        assert_eq!(
            build_base_url("b", "x", Some("https://minio.local/")),
            "https://minio.local/b/"
        );
        assert_eq!(
            build_base_url("b", "x", Some("https://{bucket}.cdn.example")),
            "https://b.cdn.example/"
        );
    }

    #[test]
    fn public_url_custom_domain_not_encoded() {
        assert_eq!(
            generate_public_url(
                "trip/照片.jpg",
                "https://b.s3.us-east-1.amazonaws.com/",
                Some("https://cdn.example")
            ),
            "https://cdn.example/trip/照片.jpg"
        );
        assert_eq!(
            generate_public_url(
                "trip/a b.jpg",
                "https://b.s3.us-east-1.amazonaws.com/",
                None
            ),
            "https://b.s3.us-east-1.amazonaws.com/trip/a%20b.jpg"
        );
    }

    #[test]
    fn endpoint_resolution() {
        assert_eq!(
            resolve_endpoint("b", "us-east-1", None).unwrap(),
            (
                "https".into(),
                "b.s3.us-east-1.amazonaws.com".into(),
                "".into()
            )
        );
        assert_eq!(
            resolve_endpoint("b", "x", Some("https://minio.local:9000")).unwrap(),
            ("https".into(), "minio.local:9000".into(), "/b".into())
        );
    }

    #[test]
    fn pagination_thresholds() {
        assert!(should_paginate(None));
        assert!(!should_paginate(Some(1000)));
        assert!(!should_paginate(Some(500)));
        assert!(should_paginate(Some(1001)));
        assert_eq!(page_size(None, 0), 1000);
        assert_eq!(page_size(Some(500), 0), 500);
        assert_eq!(page_size(Some(2500), 1000), 1000);
        assert_eq!(page_size(Some(2500), 2000), 500);
    }

    #[test]
    fn canonical_query_sorted_encoded() {
        let q = canonical_query(&[
            ("prefix".into(), "a/b c".into()),
            ("list-type".into(), "2".into()),
            ("max-keys".into(), "1000".into()),
        ]);
        // 按 key 排序：list-type, max-keys, prefix；空格编码 %20，'/' 编码 %2F
        assert_eq!(q, "list-type=2&max-keys=1000&prefix=a%2Fb%20c");
    }

    #[test]
    fn parse_list_result_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
          <Name>bucket</Name>
          <IsTruncated>true</IsTruncated>
          <NextContinuationToken>TOKEN123</NextContinuationToken>
          <Contents>
            <Key>trip/a.jpg</Key>
            <LastModified>2024-01-02T03:04:05.000Z</LastModified>
            <ETag>"abc123"</ETag>
            <Size>2048</Size>
          </Contents>
          <Contents>
            <Key>b.png</Key>
            <LastModified>2024-02-02T00:00:00.000Z</LastModified>
            <ETag>"def"</ETag>
            <Size>10</Size>
          </Contents>
        </ListBucketResult>"#;
        let r = parse_list_xml(xml).unwrap();
        assert!(r.is_truncated);
        assert_eq!(r.next_continuation_token.as_deref(), Some("TOKEN123"));
        assert_eq!(r.contents.len(), 2);
        let objs: Vec<StorageObject> = r.contents.into_iter().map(to_object).collect();
        assert_eq!(objs[0].key, "trip/a.jpg");
        assert_eq!(objs[0].size, Some(2048));
        assert_eq!(objs[0].etag.as_deref(), Some("abc123")); // 去引号
        assert!(objs[0].last_modified.is_some());
    }
}
