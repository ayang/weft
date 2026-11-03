use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use loco_rs::prelude::*;
use uuid::Uuid;

use crate::models::medias;

/// Set once at boot from `settings.media.upload_dir` so uploads, the public
/// `/media/{filename}` route and background tasks all agree on one directory.
static UPLOAD_DIR: OnceLock<PathBuf> = OnceLock::new();

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_UPLOAD_BYTES: usize = MAX_VIDEO_BYTES + 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct MediaKind {
    pub kind: &'static str,
    pub mime: &'static str,
    pub ext: &'static str,
}

/// Validate a media upload by sniffing file content, not trusting the filename.
///
/// Browsers and image sites frequently save a JPEG/WebP with a `.png` name.
/// The bytes are authoritative; the detected type decides the stored extension.
///
/// # Errors
/// Unsupported content, bad signature, or oversized file.
pub fn inspect(_filename: &str, bytes: &[u8]) -> Result<MediaKind> {
    if bytes.is_empty() {
        return Err(Error::string("文件为空"));
    }
    let kind = sniff(bytes).ok_or_else(|| {
        Error::string("无法识别文件内容；支持 JPG/PNG/WebP/GIF/AVIF 图片和 MP4/WebM 视频")
    })?;
    let max = if kind.kind == "image" {
        MAX_IMAGE_BYTES
    } else {
        MAX_VIDEO_BYTES
    };
    if bytes.len() > max {
        return Err(Error::string(if kind.kind == "image" {
            "图片不能超过 8 MB"
        } else {
            "视频不能超过 12 MB"
        }));
    }
    Ok(kind)
}

fn sniff(bytes: &[u8]) -> Option<MediaKind> {
    let kind = |kind, mime, ext| MediaKind { kind, mime, ext };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(kind("image", "image/png", "png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(kind("image", "image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(kind("image", "image/gif", "gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(kind("image", "image/webp", "webp"));
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some(kind("video", "video/webm", "webm"));
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Some(kind("image", "image/avif", "avif"));
        }
        // Most remaining ISO BMFF brands (isom, mp42, avc1, M4V, dash...)
        // are valid MP4 containers. MOV/quicktime is accepted as MP4 and
        // served as video/mp4.
        return Some(kind("video", "video/mp4", "mp4"));
    }
    None
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
    match Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn truncate_name(name: &str) -> String {
    let name = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("upload");
    crate::util::truncate_chars(name, 120)
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
}
