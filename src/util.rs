use pulldown_cmark::{html, Event, Options, Parser, Tag};
use sha2::{Digest, Sha256};

/// Turn Markdown into safe-ish HTML. Raw HTML is escaped: authors write
/// Markdown, and LLM output is inserted only after a human review. This keeps
/// the renderer from being an XSS sink even if a model emits `<script>`.
#[must_use]
pub fn render_markdown(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SMART_PUNCTUATION;
    let parser = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_link(&dest_url).into(),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_link(&dest_url).into(),
            title,
            id,
        }),
        other => other,
    });
    let mut out = String::with_capacity(markdown.len() + 128);
    html::push_html(&mut out, parser);
    out
}

/// Rough reading time; CJK characters count faster than Latin words.
#[must_use]
pub fn reading_minutes(markdown: &str) -> i64 {
    let mut latin_words = 0_u64;
    let mut cjk_chars = 0_u64;
    for word in markdown.split_whitespace() {
        for ch in word.chars() {
            if is_cjk(ch) {
                cjk_chars += 1;
            }
        }
        if word.chars().any(|ch| ch.is_ascii_alphanumeric()) {
            latin_words += 1;
        }
    }
    let units = latin_words + cjk_chars.div_ceil(2);
    i64::try_from(units.div_ceil(200).max(1)).unwrap_or(1)
}

fn safe_link(url: &str) -> String {
    let lower = url.trim().to_ascii_lowercase();
    let allowed = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('/')
        || lower.starts_with('#')
        || lower.starts_with("./")
        || lower.starts_with("../");
    if allowed {
        url.to_string()
    } else {
        "#".to_string()
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff
    )
}

/// URL-safe slug. Latin text is lowercased and hyphenated; CJK characters are
/// kept so Chinese titles still produce readable slugs.
#[must_use]
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            dash = false;
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        format!("entry-{}", chrono::Utc::now().timestamp())
    } else {
        out
    }
}

/// Split Markdown into overlapping chunks for keyword/vector retrieval.
#[must_use]
pub fn chunk_text(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(200);
    let overlap = overlap.min(max_chars / 2);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        let trimmed = chunk.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }
    chunks
}

/// A quoted SHA-256 ETag. Loco's ETag middleware compares the response header
/// to `If-None-Match` and turns a match into a 304.
#[must_use]
pub fn etag_for(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("\"{}\"", hex(&digest))
}

#[must_use]
pub fn etag_for_str(value: &str) -> String {
    etag_for(value.as_bytes())
}

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[must_use]
pub fn truncate_chars(value: &str, max: usize) -> String {
    let mut out: String = value.chars().take(max).collect();
    if value.chars().count() > max {
        out.push('…');
    }
    out
}

#[must_use]
pub fn format_date(dt: &chrono::DateTime<chrono::FixedOffset>) -> String {
    dt.format("%Y-%m-%d").to_string()
}

/// Constant-time-ish comparison for short tokens. Length is leaked, values are
/// not compared with early exit.
#[must_use]
pub fn secure_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// RFC 7231 IMF-fixdate for `Last-Modified`.
#[must_use]
pub fn http_date(dt: &chrono::DateTime<chrono::FixedOffset>) -> String {
    dt.with_timezone(&chrono::Utc)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// Remove a leading `# Title` that duplicates the article title rendered by
/// the page chrome. Existing AI drafts often repeat the title inside body_md;
/// the stored Markdown is left untouched and only the public HTML is cleaned.
#[must_use]
pub fn strip_duplicate_title(markdown: &str, title: &str) -> String {
    let title = title.trim();
    let mut out = String::new();
    let mut started = false;
    for line in markdown.lines() {
        if !started {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(heading) = trimmed.strip_prefix("# ") {
                if heading.trim() == title {
                    started = true;
                    continue;
                }
            }
            started = true;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_raw_html() {
        let html = render_markdown("# hi\n\n<script>alert(1)</script>\n");
        assert!(html.contains("<h1>hi</h1>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        let link = render_markdown("[bad](javascript:alert(1))");
        assert!(!link.contains("javascript:"));
        assert!(link.contains("href=\"#\""));
    }

    #[test]
    fn slugify_handles_cjk() {
        assert_eq!(slugify("Hello, Loco!"), "hello-loco");
        assert_eq!(slugify("你好，世界"), "你好世界");
    }

    #[test]
    fn chunks_cover_text() {
        let chunks = chunk_text(&"a".repeat(1200), 500, 50);
        assert!(chunks.len() >= 3);
    }

    #[test]
    fn strips_only_matching_leading_title() {
        assert_eq!(strip_duplicate_title("# 标题\n\n正文", "标题"), "正文");
        assert_eq!(
            strip_duplicate_title("# 另一个标题\n\n正文", "标题"),
            "# 另一个标题\n\n正文"
        );
    }
}
