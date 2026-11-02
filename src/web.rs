use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use loco_rs::prelude::*;
use regex::Regex;
use scraper::{Html, Selector};
use serde::Serialize;

const MAX_PAGE_BYTES: usize = 1_000_000;
const MAX_TEXT_CHARS: usize = 24_000;
const MAX_REDIRECTS: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub text: String,
}

/// Fetch a public web page and reduce it to readable text.
///
/// This is intentionally server-side only and is used by the admin AI editor.
/// Private/loopback hosts are rejected before and after redirects.
///
/// # Errors
/// Invalid URL, private host, network failure, unsupported content type, or
/// oversized response.
pub async fn fetch_page(raw: &str) -> Result<FetchedPage> {
    let mut url = validate_url(raw)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("weft-reference-reader/1.0")
        .build()
        .map_err(|err| Error::string(&format!("reference client failed: {err}")))?;

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|err| Error::string(&format!("抓取网页失败 {url}: {err}")))?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(Error::string("网页重定向次数过多"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| Error::string("网页重定向缺少 Location"))?;
            url = validate_url(
                url.join(location)
                    .map_err(|_| Error::string("网页重定向地址无效"))?
                    .as_str(),
            )?;
            continue;
        }
        if !response.status().is_success() {
            return Err(Error::string(&format!(
                "抓取网页失败 {url}: HTTP {}",
                response.status()
            )));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_PAGE_BYTES as u64 {
                return Err(Error::string("网页超过 1 MB 限制"));
            }
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !(content_type.contains("text/html")
            || content_type.contains("text/plain")
            || content_type.contains("application/xhtml+xml"))
        {
            return Err(Error::string(&format!(
                "暂不支持该网页类型：{content_type}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|err| Error::string(&format!("读取网页失败：{err}")))?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(Error::string("网页超过 1 MB 限制"));
        }
        let raw_text = String::from_utf8_lossy(&bytes);
        let (title, text) = if content_type.contains("html") {
            extract_html(&raw_text)
        } else {
            (url.to_string(), raw_text.to_string())
        };
        let text = truncate_chars(&normalize_whitespace(&text), MAX_TEXT_CHARS);
        if text.trim().is_empty() {
            return Err(Error::string("网页没有可读正文"));
        }
        return Ok(FetchedPage {
            url: url.to_string(),
            title: if title.trim().is_empty() {
                url.host_str().unwrap_or(url.as_str()).to_string()
            } else {
                truncate_chars(&normalize_whitespace(&title), 160)
            },
            text,
        });
    }
    Err(Error::string("网页重定向次数过多"))
}

fn validate_url(raw: &str) -> Result<reqwest::Url> {
    let trimmed = raw.trim();
    if trimmed.len() > 2_048 {
        return Err(Error::string("网页链接过长"));
    }
    let url = reqwest::Url::parse(trimmed).map_err(|_| Error::string("网页链接格式无效"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::string("网页链接必须是 http/https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::string("网页链接不能包含用户名或密码"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::string("网页链接缺少域名"))?;
    validate_public_host(host, url.port_or_known_default().unwrap_or(80))?;
    Ok(url)
}

fn validate_public_host(host: &str, port: u16) -> Result<()> {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err(Error::string("不允许访问内网/本地地址"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(Error::string("不允许访问内网/本地地址"));
        }
        return Ok(());
    }
    let addrs = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
        .map_err(|_| Error::string("无法解析网页域名"))?;
    let mut found = false;
    for addr in addrs {
        found = true;
        if !is_public_ip(addr.ip()) {
            return Err(Error::string("域名解析到内网/本地地址，已拒绝"));
        }
    }
    if !found {
        return Err(Error::string("无法解析网页域名"));
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.octets()[0] == 0
                || ip.octets()[0] == 127
                || (ip.octets()[0] == 169 && ip.octets()[1] == 254)
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

fn extract_html(html: &str) -> (String, String) {
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    let script = SCRIPT.get_or_init(|| {
        Regex::new(r"(?is)<(script|style|noscript|svg|iframe)[^>]*>.*?</(script|style|noscript|svg|iframe)>")
            .expect("valid html cleanup regex")
    });
    let cleaned = script.replace_all(html, " ");
    let document = Html::parse_document(&cleaned);
    let title = Selector::parse("title")
        .ok()
        .and_then(|selector| document.select(&selector).next())
        .map(|element| element.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let selector = Selector::parse("h1, h2, h3, h4, p, li, blockquote, pre, td, th").ok();
    let mut parts = Vec::new();
    if let Some(selector) = selector {
        for element in document.select(&selector) {
            let text = normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "));
            if text.len() > 1 {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        if let Ok(selector) = Selector::parse("body") {
            if let Some(body) = document.select(&selector).next() {
                parts.push(normalize_whitespace(
                    &body.text().collect::<Vec<_>>().join(" "),
                ));
            }
        }
    }
    (title, parts.join("\n\n"))
}

#[must_use]
pub fn html_to_text(html: &str) -> String {
    let (_, text) = extract_html(html);
    text
}

fn normalize_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" \n ", "\n")
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut result = value.chars().take(max).collect::<String>();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_reader_removes_scripts_and_keeps_blocks() {
        let html = r#"<html><head><title>Page</title><script>evil()</script></head>
        <body><h1>Heading</h1><p>Hello <strong>world</strong>.</p><style>.x{}</style></body></html>"#;
        let (title, text) = extract_html(html);
        assert_eq!(title, "Page");
        assert!(text.contains("Heading"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("evil"));
        assert!(!text.contains(".x{}"));
    }

    #[test]
    fn private_hosts_are_rejected() {
        assert!(validate_url("http://127.0.0.1/").is_err());
        assert!(validate_url("http://localhost/").is_err());
        assert!(validate_url("http://10.0.0.1/").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("http://user:pass@example.com/").is_err());
    }
}
