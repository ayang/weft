use std::io::Read;

use loco_rs::prelude::*;
use serde::Serialize;

use crate::web;

const MAX_MATERIAL_BYTES: usize = 2_000_000;
const MAX_MATERIAL_CHARS: usize = 40_000;

#[derive(Debug, Clone, Serialize)]
pub struct Material {
    pub name: String,
    pub text: String,
    pub chars: usize,
}

/// Parse one uploaded editing material into plain text.
///
/// # Errors
/// Too large, unsupported/binary file, or PDF extraction failure.
pub fn parse_material(filename: &str, bytes: &[u8]) -> Result<Material> {
    if bytes.is_empty() {
        return Err(Error::string(&format!("材料 {filename} 为空")));
    }
    if bytes.len() > MAX_MATERIAL_BYTES {
        return Err(Error::string(&format!("材料 {filename} 超过 2 MB")));
    }
    let lower = filename.to_ascii_lowercase();
    let text = if lower.ends_with(".pdf") {
        let extracted = pdf_extract::extract_text_from_mem(bytes)
            .map_err(|err| Error::string(&format!("无法解析 PDF {filename}: {err}")))?;
        normalize(&extracted)
    } else if lower.ends_with(".docx") {
        let cursor = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|err| Error::string(&format!("无法解析 DOCX {filename}: {err}")))?;
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .map_err(|err| Error::string(&format!("DOCX 缺少正文：{err}")))?
            .take(10_000_000)
            .read_to_string(&mut xml)
            .map_err(|err| Error::string(&format!("读取 DOCX 失败：{err}")))?;
        normalize(&strip_docx_xml(&xml))
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        let raw = std::str::from_utf8(bytes)
            .map_err(|_| Error::string(&format!("材料 {filename} 不是 UTF-8 文本")))?;
        normalize(&web::html_to_text(raw))
    } else {
        let raw = std::str::from_utf8(bytes)
            .map_err(|_| Error::string(&format!("材料 {filename} 不是 UTF-8 文本")))?;
        if raw.contains('\0') {
            return Err(Error::string(&format!(
                "材料 {filename} 是二进制文件；请上传 PDF/DOCX/文本/Markdown/HTML"
            )));
        }
        normalize(raw)
    };
    if text.trim().is_empty() {
        return Err(Error::string(&format!("材料 {filename} 没有可读文字")));
    }
    let text = truncate(text, MAX_MATERIAL_CHARS);
    Ok(Material {
        name: filename.to_string(),
        chars: text.chars().count(),
        text,
    })
}

fn strip_docx_xml(xml: &str) -> String {
    let paragraphs = regex::Regex::new(r"</w:p>")
        .expect("valid docx paragraph regex")
        .replace_all(xml, "\n")
        .to_string();
    let tags = regex::Regex::new(r"<[^>]+>")
        .expect("valid docx tag regex")
        .replace_all(&paragraphs, " ")
        .to_string();
    let tags = regex::Regex::new(r"[ \t]+")
        .expect("valid docx whitespace regex")
        .replace_all(&tags, " ")
        .to_string();
    tags.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn normalize(value: &str) -> String {
    value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn truncate(value: String, max: usize) -> String {
    if value.chars().count() <= max {
        return value;
    }
    let mut out = value.chars().take(max).collect::<String>();
    out.push_str("\n…（材料已截断）");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_markdown_and_html() {
        let md = parse_material("notes.md", b"# Title\n\nText").unwrap();
        assert_eq!(md.name, "notes.md");
        assert!(md.text.contains("Title"));

        let html = parse_material("page.html", b"<h1>Heading</h1><p>Body</p>").unwrap();
        assert!(html.text.contains("Heading"));
        assert!(html.text.contains("Body"));
        assert!(!html.text.contains("<p>"));
    }

    #[test]
    fn parses_docx_text() {
        use std::io::{Cursor, Write};

        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t> DOCX</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let material = parse_material("notes.docx", &bytes).unwrap();
        assert!(material.text.contains("Hello DOCX"));
        assert!(material.text.contains("Second paragraph"));
    }

    #[test]
    fn rejects_binary() {
        assert!(parse_material("x.bin", b"\0\0\0").is_err());
    }
}
