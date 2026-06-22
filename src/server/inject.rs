use regex::Regex;

/// 把当前数据注入预构建 SPA 的 index.html：
/// - 先填充 server-serve 残留的 EJS 占位符 `<%- title %>` / `<%- description %>`（散落在
///   `<title>`、`apple-mobile-web-app-title`、splash 启动屏等多处），未配置则为空串，
///   并清掉其余未知 `<%- ... %>`，确保不向用户透出占位符字面量。
/// - `<script id="manifest"></script>` ← `window.__MANIFEST__ = <manifest_json>;`
/// - `<script id="config">...</script>` ← `window.__CONFIG__ = <config_json>;window.__SITE_CONFIG__ = <site_config_json>`
/// - `<title>` / `<meta name="description">` 再以站点配置权威覆盖（即便 dist 烘入了真实值）。
pub fn inject(
    html: &str,
    manifest_json: &str,
    config_json: &str,
    site_config_json: &str,
    title: Option<&str>,
    description: Option<&str>,
) -> String {
    // EJS 占位符先在原始模板上处理（在注入 manifest/config JSON 之前，避免误伤 JSON 内容）
    let filled = fill_ejs_placeholders(html, title.unwrap_or(""), description.unwrap_or(""));
    let mut out = replace_script_by_id(
        &filled,
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

/// 填充 EJS 插值占位符并清理残留：
/// - `<%- title %>` / `<%- description %>`（容忍内部空白）→ 站点配置值（HTML 转义）；
/// - 其余未知 `<%- name %>` / `<%= name %>` → 删除（静态 serve 不经 EJS，残留会原样显示）。
fn fill_ejs_placeholders(html: &str, title: &str, description: &str) -> String {
    let mut out = html.to_string();
    for (name, val) in [("title", title), ("description", description)] {
        let re = Regex::new(&format!(r"<%-\s*{name}\s*%>")).unwrap();
        let esc = html_escape(val);
        out = re
            .replace_all(&out, |_: &regex::Captures| esc.clone())
            .into_owned();
    }
    // 兜底：清掉其余未知插值占位符（仅匹配 `<%- name %>` / `<%= name %>` 形式，不碰控制语句）
    let leftover = Regex::new(r"<%[-=]\s*[\w.]+\s*%>").unwrap();
    leftover.replace_all(&out, "").into_owned()
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

    const EJS_TEMPLATE: &str = r#"<html><head>
<meta name="description" content="<%- description %>" />
<meta name="apple-mobile-web-app-title" content="<%- title %>" />
<title><%- title %></title>
<meta name="og:image" content="<%- ogImage %>" />
<script id="config">window.__CONFIG__ = {}</script>
<script id="manifest"></script>
</head><body><h1><%- title %></h1><p><%-description%></p></body></html>"#;

    #[test]
    fn fills_all_ejs_placeholders_and_strips_unknown() {
        let out = inject(EJS_TEMPLATE, "{}", "{}", "{}", Some("My Gallery"), Some("Desc & <b>"));
        // 全部 title/description 占位符（含 apple-meta、splash <h1>/<p>、紧凑写法）被填充
        assert_eq!(out.matches("My Gallery").count(), 3);
        assert!(out.contains(r#"content="Desc &amp; &lt;b&gt;""#)); // 属性里 HTML 转义
        assert!(out.contains("<p>Desc &amp; &lt;b&gt;</p>"));
        // 未知占位符 <%- ogImage %> 被清掉，且不留任何 EJS 字面量
        assert!(!out.contains("<%"));
        assert!(!out.contains("ogImage"));
    }

    #[test]
    fn ejs_placeholders_blanked_when_unset() {
        // 未配置（None）时占位符不得原样透出，应为空串
        let out = inject(EJS_TEMPLATE, "{}", "{}", "{}", None, None);
        assert!(!out.contains("<%"));
        assert!(out.contains("<title></title>"));
        assert!(out.contains("<h1></h1>"));
    }
}
