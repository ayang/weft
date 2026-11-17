use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use loco_rs::prelude::*;
use regex::Regex;
use serde::Serialize;
use uuid::Uuid;

use crate::models::{chart_specs, medias, posts, site_settings};

/// Set once at boot from `settings.media.upload_dir` so uploads, the public
/// `/media/{filename}` route and background tasks all agree on one directory.
static UPLOAD_DIR: OnceLock<PathBuf> = OnceLock::new();

pub const KIND_IMAGE: &str = "image";
pub const KIND_VIDEO: &str = "video";
pub const KIND_FILE: &str = "file";

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: usize = 12 * 1024 * 1024;
/// Attachments are served with `Content-Disposition: attachment`, so their
/// bytes never become an inline same-origin document. The size stays below the
/// 16 MB server body limit in `config/*.yaml`.
pub const MAX_ATTACHMENT_BYTES: usize = 15 * 1024 * 1024;
pub const MAX_UPLOAD_BYTES: usize = MAX_ATTACHMENT_BYTES;

/// Content types that a browser could render as an active same-origin
/// document. Attachments with these extensions are refused; ZIP them instead.
const UNSAFE_ATTACHMENT_EXTENSIONS: [&str; 21] = [
    "html", "htm", "xhtml", "xht", "svg", "svgz", "xml", "xsl", "xslt", "js", "mjs", "cjs", "php",
    "phtml", "phar", "jsp", "jspx", "asp", "aspx", "cgi", "hta",
];

#[derive(Debug, Clone)]
pub struct MediaKind {
    pub kind: &'static str,
    pub mime: &'static str,
    pub ext: String,
}

/// Validate an upload by sniffing file content first, not trusting the filename.
///
/// Browsers and image sites frequently save a JPEG/WebP with a `.png` name.
/// The bytes are authoritative; the detected type decides the stored extension.
/// Anything that is not an image or video becomes an attachment, as long as
/// its extension is not one a browser would execute inline.
///
/// # Errors
/// Empty, corrupt media, unsafe attachment type, or oversized file.
pub fn inspect(filename: &str, bytes: &[u8]) -> Result<MediaKind> {
    if bytes.is_empty() {
        return Err(Error::string("文件为空"));
    }
    if let Some(kind) = sniff(bytes) {
        let max = if kind.kind == KIND_IMAGE {
            MAX_IMAGE_BYTES
        } else {
            MAX_VIDEO_BYTES
        };
        if bytes.len() > max {
            return Err(Error::string(if kind.kind == KIND_IMAGE {
                "图片不能超过 8 MB"
            } else {
                "视频不能超过 12 MB"
            }));
        }
        return Ok(kind);
    }
    // A broken image must not silently become a downloadable attachment.
    if let Some(ext) = extension_of(filename) {
        if matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "mp4" | "webm"
        ) {
            return Err(Error::string(
                "无法识别图片/视频内容；文件可能已损坏，或请改名后再作为附件上传",
            ));
        }
    }
    let ext = attachment_ext(filename)?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(Error::string("附件不能超过 15 MB"));
    }
    Ok(MediaKind {
        kind: KIND_FILE,
        mime: mime_for_attachment(&ext),
        ext,
    })
}

fn sniff(bytes: &[u8]) -> Option<MediaKind> {
    let kind = |kind, mime, ext: &str| MediaKind {
        kind,
        mime,
        ext: ext.to_string(),
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(kind(KIND_IMAGE, "image/png", "png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(kind(KIND_IMAGE, "image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(kind(KIND_IMAGE, "image/gif", "gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(kind(KIND_IMAGE, "image/webp", "webp"));
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some(kind(KIND_VIDEO, "video/webm", "webm"));
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Some(kind(KIND_IMAGE, "image/avif", "avif"));
        }
        // Most remaining ISO BMFF brands (isom, mp42, avc1, M4V, dash...)
        // are valid MP4 containers. MOV/quicktime is accepted as MP4 and
        // served as video/mp4.
        return Some(kind(KIND_VIDEO, "video/mp4", "mp4"));
    }
    None
}

fn extension_of(filename: &str) -> Option<String> {
    let name = Path::new(filename).file_name()?.to_str()?;
    let (_, ext) = name.rsplit_once('.')?;
    let ext = ext.trim().to_ascii_lowercase();
    (!ext.is_empty()).then_some(ext)
}

fn attachment_ext(filename: &str) -> Result<String> {
    let Some(ext) = extension_of(filename) else {
        return Ok("bin".to_string());
    };
    if ext.len() > 16 || !ext.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(Error::string("附件扩展名无效；请使用带扩展名的文件名"));
    }
    if UNSAFE_ATTACHMENT_EXTENSIONS.contains(&ext.as_str()) {
        return Err(Error::string(
            "出于安全考虑，不能直接上传 HTML/SVG/脚本类附件；请打包成 ZIP 后上传",
        ));
    }
    Ok(ext)
}

/// # Errors
/// Filesystem or database failure.
pub async fn store(
    db: &DatabaseConnection,
    original_name: &str,
    bytes: &[u8],
) -> Result<medias::Model> {
    let kind = inspect(original_name, bytes)?;
    let filename = format!("{}.{}", Uuid::new_v4(), kind.ext);
    let dir = upload_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| Error::string(&format!("创建媒体目录失败：{err}")))?;
    tokio::fs::write(dir.join(&filename), bytes)
        .await
        .map_err(|err| Error::string(&format!("写入媒体失败：{err}")))?;
    let original = truncate_name(original_name);
    medias::Model::create_media(
        db,
        &filename,
        &original,
        kind.mime,
        kind.kind,
        i64::try_from(bytes.len()).unwrap_or(i64::MAX),
    )
    .await
    .map_err(Error::from)
}

/// Point media storage at the configured directory.
///
/// Called from `App::after_context`. The fallback keeps `cargo test` and
/// one-off tasks working when no app context has booted.
pub fn init_upload_dir(dir: impl Into<PathBuf>) {
    let _ = UPLOAD_DIR.set(dir.into());
}

pub fn upload_dir() -> PathBuf {
    UPLOAD_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("media").join("uploads"))
}

pub fn public_url(filename: &str) -> String {
    format!("/media/{filename}")
}

#[must_use]
pub fn markdown_alt(name: &str) -> String {
    name.replace(['[', ']', '\n', '\r'], " ")
}

pub fn video_marker(filename: &str) -> String {
    format!("[[video:{filename}]]")
}

/// Markdown-ish marker for a downloadable attachment. The label is optional;
/// it is stripped of the marker delimiters so parsing stays unambiguous.
#[must_use]
pub fn attachment_marker(filename: &str, label: &str) -> String {
    let cleaned = label.replace(['|', ']', '\n', '\r'], " ");
    let label = crate::util::truncate_chars(cleaned.trim(), 120);
    if label.is_empty() {
        format!("[[file:{filename}]]")
    } else {
        format!("[[file:{filename}|{label}]]")
    }
}

/// The exact text an editor should insert for a stored media row.
#[must_use]
pub fn markdown_for(item: &medias::Model) -> String {
    match item.kind.as_str() {
        KIND_IMAGE => format!(
            "![{}]({})",
            markdown_alt(&item.original_name),
            public_url(&item.filename)
        ),
        KIND_VIDEO => video_marker(&item.filename),
        _ => attachment_marker(&item.filename, &item.original_name),
    }
}

#[must_use]
pub fn download_url(filename: &str) -> String {
    format!("/media/{filename}/download")
}

/// One place where a stored media file is referenced. `kind` is stable for
/// clients and the label/URL are ready for an admin UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaUsage {
    pub kind: &'static str,
    pub label: String,
    pub url: Option<String>,
}

/// Scan every place a media file can be referenced: post bodies (Markdown,
/// video and attachment markers), post covers, the homepage hero and chart
/// render/data configs. Any post status counts, so drafts do not lose images.
///
/// # Errors
/// Database failure.
pub async fn usage_index(db: &DatabaseConnection) -> Result<HashMap<String, Vec<MediaUsage>>> {
    let mut index: HashMap<String, Vec<MediaUsage>> = HashMap::new();

    let post_rows = posts::Entity::find().all(db).await?;
    for post in &post_rows {
        let edit_url = Some(format!("/admin/posts/{}/edit", post.id));
        collect_usage(
            &post.body_md,
            &MediaUsage {
                kind: "post",
                label: post.title.clone(),
                url: edit_url.clone(),
            },
            &mut index,
        );
        if let Some(cover) = post.cover_image.as_deref() {
            collect_usage(
                cover,
                &MediaUsage {
                    kind: "cover",
                    label: format!("封面 · {}", post.title),
                    url: edit_url,
                },
                &mut index,
            );
        }
    }

    if let Some(settings) = site_settings::Model::singleton(db).await? {
        if let Some(hero) = settings.hero_image.as_deref() {
            collect_usage(
                hero,
                &MediaUsage {
                    kind: "hero",
                    label: "站点设置 · 首页 Banner".to_string(),
                    url: Some("/admin/settings/site".to_string()),
                },
                &mut index,
            );
        }
    }

    let chart_rows = chart_specs::Entity::find().all(db).await?;
    for chart in &chart_rows {
        let usage = MediaUsage {
            kind: "chart",
            label: format!("图表 · {}", chart.title),
            url: Some(format!("/admin/charts/{}/edit", chart.id)),
        };
        collect_usage(&chart.render_config.to_string(), &usage, &mut index);
        collect_usage(&chart.data_source_config.to_string(), &usage, &mut index);
    }

    Ok(index)
}

fn collect_usage(text: &str, usage: &MediaUsage, index: &mut HashMap<String, Vec<MediaUsage>>) {
    for captures in media_reference_regex().captures_iter(text) {
        let Some(captured) = captures.get(1) else {
            continue;
        };
        let filename = captured.as_str();
        if !safe_filename(filename) {
            continue;
        }
        let entry = index.entry(filename.to_string()).or_default();
        if !entry.contains(usage) {
            entry.push(usage.clone());
        }
    }
}

fn media_reference_regex() -> &'static Regex {
    static REFERENCE: OnceLock<Regex> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        Regex::new(r"(?:/media/|\[\[(?:video|file):)([A-Za-z0-9._-]{1,120})")
            .expect("media reference regex is valid")
    })
}

/// Remove the stored file for a media row. A missing file still counts as
/// removed so a stale database row can be cleaned up.
///
/// # Errors
/// Unsafe stored filename or a filesystem error other than `NotFound`.
pub async fn remove_file(filename: &str) -> Result<()> {
    if !safe_filename(filename) {
        return Err(Error::string("媒体文件名不合法"));
    }
    match tokio::fs::remove_file(upload_dir().join(filename)).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::string(&format!("删除媒体文件失败：{err}"))),
    }
}

/// Uppercase extension shown on attachment cards, e.g. `PDF`.
#[must_use]
pub fn type_label(filename: &str) -> String {
    extension_of(filename).map_or_else(
        || "FILE".to_string(),
        |ext| crate::util::truncate_chars(&ext, 8).to_ascii_uppercase(),
    )
}

#[must_use]
pub fn human_size(size: i64) -> String {
    let size = size.max(0);
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else if size >= 1024 {
        format!("{} KB", (size + 512) / 1024)
    } else {
        format!("{size} B")
    }
}

/// Build a safe `Content-Disposition` value for an attachment download.
/// The ASCII fallback keeps old clients working; `filename*` preserves the
/// original non-ASCII name.
#[must_use]
pub fn content_disposition(name: &str) -> String {
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_filename(name),
        rfc5987_encode(name)
    )
}

fn ascii_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    let stem = out.split('.').next().unwrap_or_default();
    if stem.is_empty() {
        let ext = out.rsplit_once('.').map_or("", |(_, ext)| ext);
        if ext.is_empty() {
            "download".to_string()
        } else {
            format!("download.{ext}")
        }
    } else {
        out.to_string()
    }
}

fn rfc5987_encode(value: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    if out.is_empty() {
        "download".to_string()
    } else {
        out
    }
}

#[must_use]
pub fn safe_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 120
        && filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !filename.starts_with('.')
        && !filename.contains("..")
}

#[must_use]
pub fn mime_for_filename(filename: &str) -> &'static str {
    let ext = extension_of(filename).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        other if UNSAFE_ATTACHMENT_EXTENSIONS.contains(&other) => "application/octet-stream",
        other => mime_for_attachment(other),
    }
}

/// Whether a stored file is safe and useful to render inline in a page.
#[must_use]
pub fn is_inline_media(filename: &str) -> bool {
    matches!(
        extension_of(filename).unwrap_or_default().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "mp4" | "webm"
    )
}

#[must_use]
pub fn mime_for_attachment(ext: &str) -> &'static str {
    match ext {
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        "txt" | "md" | "markdown" | "csv" | "tsv" | "log" => "text/plain",
        "json" => "application/json",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "epub" => "application/epub+zip",
        "mobi" => "application/x-mobipocket-ebook",
        "djvu" => "image/vnd.djvu",
        "ipynb" => "application/x-ipynb+json",
        "tex" => "application/x-tex",
        "bib" => "application/x-bibtex",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        _ => "application/octet-stream",
    }
}

fn truncate_name(name: &str) -> String {
    let raw = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("upload");
    let cleaned = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    let cleaned = cleaned.trim();
    let cleaned = if cleaned.is_empty() {
        "upload"
    } else {
        cleaned
    };
    crate::util::truncate_chars(cleaned, 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_and_mismatched_files() {
        assert!(inspect("a.svg", b"<svg>").is_err());
        assert!(inspect("a.png", b"not png").is_err());
        assert!(inspect("a.mp4", b"not mp4").is_err());
    }

    #[test]
    fn detects_content_even_when_extension_is_wrong() {
        let kind = inspect("cover.png", &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]).unwrap();
        assert_eq!(kind.mime, "image/jpeg");
        assert_eq!(kind.ext, "jpg");
    }

    #[test]
    fn accepts_matching_png_signature() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"payload");
        let kind = inspect("cover.png", &bytes).unwrap();
        assert_eq!(kind.kind, "image");
        assert_eq!(kind.mime, "image/png");
    }

    #[test]
    fn accepts_documents_as_attachments() {
        let kind = inspect("团队报告.PDF", b"%PDF-1.7\npayload").unwrap();
        assert_eq!(kind.kind, KIND_FILE);
        assert_eq!(kind.ext, "pdf");
        assert_eq!(kind.mime, "application/pdf");

        let kind = inspect("notes.txt", b"plain text").unwrap();
        assert_eq!(kind.kind, KIND_FILE);
        assert_eq!(kind.ext, "txt");
        assert_eq!(kind.mime, "text/plain");

        let kind = inspect("README", b"no extension").unwrap();
        assert_eq!(kind.ext, "bin");
        assert_eq!(kind.mime, "application/octet-stream");
    }

    #[test]
    fn refuses_web_active_attachments() {
        assert!(inspect("page.html", b"<!doctype html>").is_err());
        assert!(inspect("icon.svg", b"<svg></svg>").is_err());
        assert!(inspect("app.js", b"alert(1)").is_err());
        assert!(inspect("page.xhtml", b"<html></html>").is_err());
    }

    #[test]
    fn builds_attachment_markers_and_links() {
        assert_eq!(
            attachment_marker("abc.pdf", "团队报告"),
            "[[file:abc.pdf|团队报告]]"
        );
        assert_eq!(attachment_marker("abc.pdf", "a|b]"), "[[file:abc.pdf|a b]]");
        assert_eq!(attachment_marker("abc.pdf", "  "), "[[file:abc.pdf]]");
        assert_eq!(download_url("abc.pdf"), "/media/abc.pdf/download");
        assert_eq!(type_label("abc.pdf"), "PDF");
        assert_eq!(human_size(1536), "2 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn usage_regex_finds_markdown_markers_and_download_links() {
        let mut index = HashMap::new();
        let usage = MediaUsage {
            kind: "post",
            label: "标题".to_string(),
            url: Some("/admin/posts/1/edit".to_string()),
        };
        collect_usage(
            "![a](/media/a.png)\n[[video:b.mp4]]\n[[file:c.pdf|报告]]\n[下载](/media/d.pdf/download)",
            &usage,
            &mut index,
        );
        assert!(index.contains_key("a.png"));
        assert!(index.contains_key("b.mp4"));
        assert!(index.contains_key("c.pdf"));
        assert!(index.contains_key("d.pdf"));
        assert_eq!(index.get("a.png").unwrap().len(), 1);
        assert_eq!(index.get("a.png").unwrap()[0].label, "标题");

        let mut unsafe_index = HashMap::new();
        collect_usage("[[file:../secret.pdf]]", &usage, &mut unsafe_index);
        assert!(unsafe_index.is_empty());
    }

    #[test]
    fn content_disposition_keeps_a_safe_ascii_fallback() {
        let value = content_disposition("团队 报告.pdf");
        assert!(value.starts_with("attachment; filename=\""));
        assert!(value.contains("filename=\"download.pdf\""));
        assert!(value.contains("filename*=UTF-8''"));
        assert!(value.contains("%E5%9B%A2"));
        assert!(!value.contains('\n'));

        let injected = content_disposition("a\"; filename=x.pdf");
        assert!(!injected.contains('\"') || injected.matches('\"').count() == 2);
        assert!(!injected.contains("filename=x"));
    }
}
