//! 手写 AWS Signature V4（header-based），对齐上游（不依赖 aws-sdk）。
//! 仅需 HMAC-SHA256 + SHA256，纯 Rust。
#![allow(dead_code)] // M2 进行中：s3.rs 接线后这些项即被使用

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// 空载荷的 SHA-256（hex）。
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub struct SigningParams<'a> {
    pub method: &'a str,
    /// 已规范化/编码的路径（逐段编码、保留 `/`）。
    pub canonical_uri: &'a str,
    /// 已排序+编码的 query（`k=v&k2=v2`，无前导 `?`）。
    pub canonical_query: &'a str,
    /// 头列表（name 任意大小写，签名时会小写并按名排序）。
    pub headers: &'a [(String, String)],
    pub payload_sha256_hex: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    /// `YYYYMMDDTHHMMSSZ`（UTC）。
    pub amz_date: &'a str,
}

pub struct Signed {
    pub authorization: String,
    pub signature: String,
    pub signed_headers: String,
}

pub fn sign(p: &SigningParams) -> Signed {
    // canonical headers：小写名、折叠首尾空白、按名排序
    let mut hs: Vec<(String, String)> = p
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    hs.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = hs
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        p.method,
        p.canonical_uri,
        p.canonical_query,
        canonical_headers,
        signed_headers,
        p.payload_sha256_hex
    );

    let date = &p.amz_date[..8]; // YYYYMMDD
    let scope = format!("{}/{}/{}/aws4_request", date, p.region, p.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        p.amz_date,
        scope,
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac(format!("AWS4{}", p.secret_key).as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, p.region.as_bytes());
    let k_service = hmac(&k_region, p.service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        p.access_key, scope, signed_headers, signature
    );
    Signed {
        authorization,
        signature,
        signed_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // AWS SigV4 官方测试套件 "get-vanilla"
    // https://docs.aws.amazon.com/.../sigv4-create-canonical-request.html
    #[test]
    fn aws_known_answer_get_vanilla() {
        let headers = vec![
            ("Host".to_string(), "example.amazonaws.com".to_string()),
            ("X-Amz-Date".to_string(), "20150830T123600Z".to_string()),
        ];
        let p = SigningParams {
            method: "GET",
            canonical_uri: "/",
            canonical_query: "",
            headers: &headers,
            payload_sha256_hex: EMPTY_PAYLOAD_SHA256,
            region: "us-east-1",
            service: "service",
            access_key: "AKIDEXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            amz_date: "20150830T123600Z",
        };
        let signed = sign(&p);
        assert_eq!(
            signed.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(signed.signed_headers, "host;x-amz-date");
        assert!(signed.authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request"
        ));
    }

    #[test]
    fn empty_payload_constant_matches() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_SHA256);
    }
}
