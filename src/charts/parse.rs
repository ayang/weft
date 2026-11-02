use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_ROWS: usize = 1_000;
const MAX_COLUMNS: usize = 40;
const MAX_CELL_CHARS: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParsedData {
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    pub kind: String,
    #[serde(skip)]
    pub raw_text: String,
}

impl ParsedData {
    #[must_use]
    pub fn sample(&self, limit: usize) -> Vec<Value> {
        self.rows.iter().take(limit).cloned().collect()
    }
}

/// Parse an uploaded CSV/TSV/JSON/Markdown/text file into a bounded table.
/// Parsing is never delegated to the LLM for well-known tabular formats; the
/// model is only asked to choose or transform the presentation.
///
/// # Errors
/// Unsupported/binary/oversized files or malformed structured input.
pub fn parse_file(filename: &str, bytes: &[u8]) -> Result<ParsedData> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::string("文件超过 1 MiB 限制"));
    }
    if bytes.contains(&0) {
        return Err(Error::string("不支持二进制文件，请上传 UTF-8 文本"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::string("文件不是有效的 UTF-8 文本"))?;
    let lower = filename.to_ascii_lowercase();
    let hint = if lower.ends_with(".json") {
        "json"
    } else if lower.ends_with(".csv") {
        "csv"
    } else if lower.ends_with(".tsv") {
        "tsv"
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "markdown"
    } else {
        "text"
    };
    let mut parsed = parse_text(text, hint)?;
    parsed.raw_text = text.chars().take(20_000).collect();
    Ok(parsed)
}

/// # Errors
/// Malformed structured input or no usable rows.
pub fn parse_text(text: &str, hint: &str) -> Result<ParsedData> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err(Error::string("文件内容为空"));
    }
    let parsed = match hint {
        "json" => parse_json(text)?,
        "markdown" => parse_markdown_table(text)
            .or_else(|| parse_delimited(text, '\t').ok())
            .ok_or_else(|| Error::string("Markdown 表格解析失败"))?,
        "csv" => parse_delimited(text, ',')?,
        "tsv" => parse_delimited(text, '\t')?,
        _ => parse_auto(text)?,
    };
    if parsed.rows.is_empty() {
        return Err(Error::string("没有从文件中解析到数据行"));
    }
    Ok(parsed)
}

fn parse_auto(text: &str) -> Result<ParsedData> {
    if text.starts_with('{') || text.starts_with('[') {
        if let Ok(parsed) = parse_json(text) {
            return Ok(parsed);
        }
    }
    if text.lines().any(|line| line.trim_start().starts_with('|')) {
        if let Some(parsed) = parse_markdown_table(text) {
            return Ok(parsed);
        }
    }
    let sample = text.lines().take(5).collect::<Vec<_>>().join("\n");
    if sample.contains('\t') {
        return parse_delimited(text, '\t');
    }
    if sample.contains(',') {
        return parse_delimited(text, ',');
    }
    let rows = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_ROWS)
        .map(|line| {
            let mut object = Map::new();
            object.insert(
                "文本".to_string(),
                Value::String(crate::util::truncate_chars(line, MAX_CELL_CHARS)),
            );
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    Ok(ParsedData {
        columns: vec!["文本".to_string()],
        rows,
        kind: "text".to_string(),
        raw_text: String::new(),
    })
}

fn parse_json(text: &str) -> Result<ParsedData> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| Error::string(&format!("JSON 解析失败：{err}")))?;
    let (columns, rows) =
        json_rows(&value).ok_or_else(|| Error::string("JSON 中没有可用的数组/对象数据"))?;
    let mut rows = rows;
    rows.truncate(MAX_ROWS);
    Ok(ParsedData {
        columns,
        rows,
        kind: "json".to_string(),
        raw_text: String::new(),
    })
}

fn json_rows(value: &Value) -> Option<(Vec<String>, Vec<Value>)> {
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return None;
            }
            if items.iter().all(Value::is_object) {
                let columns = columns_from_objects(items);
                return Some((columns, items.clone()));
            }
            if items.iter().all(Value::is_array) {
                let first = items.first()?.as_array()?;
                let has_header = first.iter().all(|value| value.is_string());
                let columns = if has_header {
                    first
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            let name = value
                                .as_str()
                                .filter(|text| !text.trim().is_empty())
                                .map_or_else(|| format!("列{}", index + 1), str::to_string);
                            clean_column(&name)
                        })
                        .collect::<Vec<_>>()
                } else {
                    (0..first.len())
                        .map(|index| format!("列{}", index + 1))
                        .collect()
                };
                let rows = items
                    .iter()
                    .skip(usize::from(has_header))
                    .filter_map(Value::as_array)
                    .map(|row| {
                        let mut object = Map::new();
                        for (index, column) in columns.iter().enumerate() {
                            object.insert(
                                column.clone(),
                                row.get(index).cloned().map_or(Value::Null, bounded),
                            );
                        }
                        Value::Object(object)
                    })
                    .collect();
                return Some((columns, rows));
            }
            let columns = vec!["值".to_string()];
            let rows = items
                .iter()
                .map(|value| {
                    let mut object = Map::new();
                    object.insert("值".to_string(), bounded(value.clone()));
                    Value::Object(object)
                })
                .collect();
            Some((columns, rows))
        }
        Value::Object(object) => {
            for key in ["data", "rows", "items", "results"] {
                if let Some(nested) = object.get(key) {
                    if let Some(parsed) = json_rows(nested) {
                        return Some(parsed);
                    }
                }
            }
            if object.values().all(Value::is_array) {
                let columns = object.keys().cloned().collect::<Vec<_>>();
                let length = object
                    .values()
                    .filter_map(Value::as_array)
                    .map(Vec::len)
                    .max()
                    .unwrap_or(0);
                let rows = (0..length)
                    .map(|index| {
                        let mut row = Map::new();
                        for (key, values) in object {
                            let value = values
                                .as_array()
                                .and_then(|values| values.get(index))
                                .cloned()
                                .unwrap_or(Value::Null);
                            row.insert(key.clone(), bounded(value));
                        }
                        Value::Object(row)
                    })
                    .collect();
                return Some((columns, rows));
            }
            Some((object.keys().cloned().collect(), vec![value.clone()]))
        }
        _ => None,
    }
}

fn columns_from_objects(items: &[Value]) -> Vec<String> {
    let mut columns = Vec::new();
    for item in items {
        if let Some(object) = item.as_object() {
            for key in object.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    columns
}

fn parse_markdown_table(text: &str) -> Option<ParsedData> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|') && line.ends_with('|'))
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return None;
    }
    let split = |line: &str| -> Vec<String> {
        line.trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect()
    };
    let headers = split(lines[0]);
    if headers.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    for line in lines.iter().skip(1) {
        let cells = split(line);
        if cells.iter().all(|cell| {
            cell.chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
        }) {
            continue;
        }
        let mut object = Map::new();
        for (index, header) in headers.iter().enumerate() {
            object.insert(
                clean_column(header),
                cells.get(index).cloned().map_or(Value::Null, Value::String),
            );
        }
        rows.push(Value::Object(object));
        if rows.len() >= MAX_ROWS {
            break;
        }
    }
    Some(ParsedData {
        columns: headers.iter().map(|header| clean_column(header)).collect(),
        rows,
        kind: "markdown".to_string(),
        raw_text: String::new(),
    })
}

fn parse_delimited(text: &str, delimiter: char) -> Result<ParsedData> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(u8::try_from(u32::from(delimiter)).unwrap_or(b','))
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|err| Error::string(&format!("表格表头解析失败：{err}")))?
        .iter()
        .enumerate()
        .map(|(index, header)| {
            let header = header.trim();
            if header.is_empty() {
                clean_column(&format!("列{}", index + 1))
            } else {
                clean_column(header)
            }
        })
        .collect::<Vec<_>>();
    if headers.len() > MAX_COLUMNS {
        return Err(Error::string("列数超过 40 列限制"));
    }
    let mut rows = Vec::new();
    for record in reader.records().take(MAX_ROWS) {
        let record = record.map_err(|err| Error::string(&format!("表格行解析失败：{err}")))?;
        let mut object = Map::new();
        for (index, header) in headers.iter().enumerate() {
            object.insert(
                header.clone(),
                Value::String(
                    record
                        .get(index)
                        .unwrap_or_default()
                        .chars()
                        .take(MAX_CELL_CHARS)
                        .collect(),
                ),
            );
        }
        rows.push(Value::Object(object));
    }
    Ok(ParsedData {
        columns: headers,
        rows,
        kind: if delimiter == '\t' { "tsv" } else { "csv" }.to_string(),
        raw_text: String::new(),
    })
}

fn clean_column(value: &str) -> String {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        "未命名列".to_string()
    } else {
        crate::util::truncate_chars(value, 80)
    }
}

fn bounded(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(crate::util::truncate_chars(&text, MAX_CELL_CHARS)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv() {
        let parsed = parse_file("data.csv", b"month,value\nJan,3\nFeb,5\n").unwrap();
        assert_eq!(parsed.columns, vec!["month", "value"]);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[1]["value"], "5");
    }

    #[test]
    fn parses_json_and_markdown() {
        let json = parse_file(
            "data.json",
            br#"[{"label":"A","value":1},{"label":"B","value":2}]"#,
        )
        .unwrap();
        assert_eq!(json.rows.len(), 2);
        let markdown = parse_file(
            "data.md",
            "| 月份 | 数量 |\n| --- | --- |\n| 1月 | 3 |\n| 2月 | 5 |\n".as_bytes(),
        )
        .unwrap();
        assert_eq!(markdown.rows.len(), 2);
        assert_eq!(markdown.rows[0]["数量"], "3");
    }

    #[test]
    fn rejects_binary() {
        assert!(parse_file("x.csv", &[0, 1, 2]).is_err());
    }
}
