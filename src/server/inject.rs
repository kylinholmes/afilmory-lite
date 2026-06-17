use regex::Regex;

/// 把当前数据注入预构建 SPA 的 index.html：
/// - `<script id="manifest"></script>` ← `window.__MANIFEST__ = <manifest_json>;`
/// - `<script id="config">...</script>` ← `window.__CONFIG__ = <config_json>;window.__SITE_CONFIG__ = <site_config_json>`
/// - `<title>` / `<meta name="description">` 用站点配置覆盖（清理 server-serve 残留的 `<%- ... %>`）。
pub fn inject(
    html: &str,
    manifest_json: &str,
    config_json: &str,
    site_config_json: &str,
    title: Option<&str>,
    description: Option<&str>,
) -> String {
    let mut out = replace_script_by_id(
        html,
        "manifest",
        &format!("window.__MANIFEST__ = {};", js_safe(manifest_json)),
    );
    out = replace_script_by_id(
        &out,
        "config",
        &format!(
            "window.__CONFIG__ = {};window.__SITE_CONFIG__ = {}",
            js_safe(config_json),
            js_safe(site_config_json)
        ),
    );

    if let Some(t) = title {
        let re = Regex::new(r"(?s)<title>.*?</title>").unwrap();
        let rep = format!("<title>{}</title>", html_escape(t));
        out = re
            .replace(&out, |_: &regex::Captures| rep.clone())
            .into_owned();
    }
    if let Some(d) = description {
        let re = Regex::new(r#"(?s)(<meta\s+name="description"\s+content=")[^"]*(")"#).unwrap();
        let esc = html_escape(d);
        out = re
            .replace(&out, |c: &regex::Captures| {
                format!("{}{}{}", &c[1], esc, &c[2])
            })
            .into_owned();
    }
    out
}

/// 替换 `<script ... id="ID" ...>...</script>` 标签内容，保留开/闭标签与属性。
fn replace_script_by_id(html: &str, id: &str, body: &str) -> String {
    let pattern = format!(
        r#"(?s)(<script[^>]*id="{}"[^>]*>).*?(</script>)"#,
        regex::escape(id)
    );
    let re = Regex::new(&pattern).unwrap();
    re.replace(html, |c: &regex::Captures| {
        format!("{}{}{}", &c[1], body, &c[2])
    })
    .into_owned()
}

/// 防止 JSON 中的 `</script>` 撑破 script 标签（`<\/` 在 JS 字符串里仍解析为 `</`）。
fn js_safe(json: &str) -> String {
    json.replace("</", "<\\/")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = r#"<!doctype html><html><head>
<title>placeholder</title>
<meta name="description" content="old desc">
<script id="config">window.__CONFIG__ = {}</script>
<script id="manifest"></script>
</head><body></body></html>"#;

    #[test]
    fn injects_all_globals_and_meta() {
        let out = inject(
            TEMPLATE,
            r#"{"version":"v10","data":[]}"#,
            "{}",
            r#"{"name":"X"}"#,
            Some("My Title"),
            Some("My Desc"),
        );
        assert!(out.contains(r#"window.__MANIFEST__ = {"version":"v10","data":[]};"#));
        assert!(out.contains(r#"window.__CONFIG__ = {};window.__SITE_CONFIG__ = {"name":"X"}"#));
        assert!(out.contains("<title>My Title</title>"));
        assert!(out.contains(r#"content="My Desc""#));
        assert!(!out.contains("placeholder"));
        assert!(!out.contains("old desc"));
    }

    #[test]
    fn escapes_script_breakout() {
        let out = inject(TEMPLATE, r#"{"t":"a</script>b"}"#, "{}", "{}", None, None);
        assert!(out.contains(r#"a<\/script>b"#));
        // 注入内容里不得出现裸 </script>（除真正的闭合标签外）
        assert!(!out.contains("a</script>b"));
    }
}
