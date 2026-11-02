use std::collections::BTreeSet;

use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{parse::ParsedData, validate_readonly_sql};
use crate::models::chart_specs;

pub const KINDS: [&str; 5] = ["bar", "line", "area", "pie", "scatter"];
const MAX_ROWS: usize = 1_000;
const MAX_COLUMNS: usize = 40;
const MAX_CELL_CHARS: usize = 2_000;
const MAX_Y_FIELDS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartOptions {
    pub smooth: bool,
    pub stack: bool,
    pub horizontal: bool,
    pub legend: bool,
    pub labels: bool,
    pub palette: String,
    pub sort: String,
    pub height: f64,
    pub x_label: String,
    pub y_label: String,
    pub y_unit: String,
    pub number_format: String,
    pub y_min: Option<f64>,
    pub y_max: Option<f64>,
    pub log_y: bool,
    pub data_zoom: bool,
    pub legend_position: String,
    pub dual_axis: bool,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            smooth: false,
            stack: false,
            horizontal: false,
            legend: true,
            labels: false,
            palette: "default".to_string(),
            sort: "none".to_string(),
            height: 380.0,
            x_label: String::new(),
            y_label: String::new(),
            y_unit: String::new(),
            number_format: "auto".to_string(),
            y_min: None,
            y_max: None,
            log_y: false,
            data_zoom: false,
            legend_position: "top".to_string(),
            dual_axis: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ChartBlueprint {
    pub title: String,
    pub kind: String,
    pub x_field: String,
    pub y_fields: Vec<String>,
    pub options: ChartOptions,
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VisualChartInput {
    pub kind: String,
    pub x_field: String,
    pub y_fields: Vec<String>,
    pub options: ChartOptions,
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    pub source_type: String,
    pub sql_query: String,
    pub sql_limit: Option<usize>,
    pub http_url: String,
    pub http_data_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartBootstrap {
    pub id: Option<i64>,
    pub post_id: i64,
    pub title: String,
    pub intent: String,
    pub status: String,
    pub refresh_cron: String,
    pub source_type: String,
    pub sql_query: String,
    pub sql_limit: usize,
    pub http_url: String,
    pub http_data_path: String,
    pub kind: String,
    pub x_field: String,
    pub y_fields: Vec<String>,
    pub options: ChartOptions,
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    pub advanced_config: String,
    pub advanced_render: String,
    pub last_success: String,
    pub error: String,
    pub stale: bool,
    pub is_new: bool,
}

/// Convert the visual builder payload into a validated blueprint.
///
/// # Errors
/// Empty/oversized data, unknown chart kind, or no usable value columns.
pub fn normalize(input: VisualChartInput) -> Result<ChartBlueprint> {
    let blueprint = ChartBlueprint {
        title: String::new(),
        kind: input.kind,
        x_field: input.x_field,
        y_fields: input.y_fields,
        options: input.options,
        columns: input.columns,
        rows: input.rows,
    };
    normalize_blueprint(blueprint, Vec::new(), Vec::new())
}

/// # Errors
/// Empty/oversized data, unknown chart kind, or no usable value columns.
pub fn normalize_blueprint(
    mut blueprint: ChartBlueprint,
    fallback_rows: Vec<Value>,
    fallback_columns: Vec<String>,
) -> Result<ChartBlueprint> {
    if blueprint.rows.is_empty() {
        blueprint.rows = fallback_rows;
    }
    if blueprint.columns.is_empty() {
        blueprint.columns = fallback_columns;
    }
    if blueprint.rows.is_empty() {
        return Err(Error::string(
            "请先添加数据、上传文件，或让 AI 生成示例数据",
        ));
    }
    if blueprint.rows.len() > MAX_ROWS {
        return Err(Error::string(&format!(
            "数据行数过多（最大 {MAX_ROWS} 行）"
        )));
    }

    blueprint.rows = normalize_rows(&mut blueprint.columns, blueprint.rows)?;
    if blueprint.columns.len() > MAX_COLUMNS {
        blueprint.columns.truncate(MAX_COLUMNS);
    }
    if blueprint.columns.is_empty() {
        return Err(Error::string("数据没有可用的列"));
    }

    blueprint.kind = normalize_kind(&blueprint.kind);
    blueprint.options = normalize_options(blueprint.options);
    blueprint.title = crate::util::truncate_chars(blueprint.title.trim(), 120);

    if blueprint.x_field.is_empty() || !blueprint.columns.contains(&blueprint.x_field) {
        blueprint.x_field = infer_x_field(&blueprint.rows, &blueprint.columns);
    }
    blueprint
        .y_fields
        .retain(|field| blueprint.columns.contains(field) && field != &blueprint.x_field);
    blueprint.y_fields.dedup();
    if blueprint.y_fields.is_empty() {
        blueprint.y_fields =
            infer_y_fields(&blueprint.rows, &blueprint.columns, &blueprint.x_field);
    }
    if blueprint.kind == "pie" {
        blueprint.y_fields.truncate(1);
    }
    blueprint.y_fields.truncate(MAX_Y_FIELDS);
    if blueprint.y_fields.is_empty() {
        return Err(Error::string(
            "没有找到可用于纵轴的数值列，请检查数据或手动选择字段",
        ));
    }
    if blueprint.options.sort != "none" {
        sort_rows(
            &mut blueprint.rows,
            &blueprint.x_field,
            &blueprint.y_fields,
            &blueprint.options.sort,
        );
    }
    // Normalize after sorting so numbers and labels stay consistent.
    blueprint.rows = normalize_rows(&mut blueprint.columns, blueprint.rows)?;
    Ok(blueprint)
}

/// Build the ECharts option that is persisted in `chart_specs.render_config`.
///
/// # Errors
/// Unknown kind or an empty y-field selection.
pub fn to_echarts(blueprint: &ChartBlueprint) -> Result<Value> {
    let kind = normalize_kind(&blueprint.kind);
    if blueprint.y_fields.is_empty() {
        return Err(Error::string("至少选择一个纵轴字段"));
    }
    let palette = palette_colors(&blueprint.options.palette);
    let legend_show = blueprint.options.legend;
    let labels = blueprint.options.labels;
    let options = &blueprint.options;
    let mut legend = json!({"show": legend_show, "top": 0, "type": "scroll"});
    let mut grid = json!({
        "left": 48,
        "right": 24,
        "top": if legend_show { 38 } else { 16 },
        "bottom": 36,
        "containLabel": true
    });
    match options.legend_position.as_str() {
        "bottom" => {
            legend = json!({"show": legend_show, "bottom": 0, "type": "scroll"});
            grid["top"] = json!(if legend_show { 16 } else { 12 });
            grid["bottom"] = json!(if options.data_zoom { 64 } else { 52 });
        }
        "right" => {
            legend = json!({"show": legend_show, "right": 0, "top": 8, "orient": "vertical", "type": "scroll"});
            grid["right"] = json!(100);
        }
        _ => {}
    }
    let mut option = json!({
        "animationDuration": 420,
        "color": palette,
        "dataset": {"source": []},
        "tooltip": {"trigger": if kind == "pie" { "item" } else { "axis" }},
        "legend": legend,
        "grid": grid.clone(),
        "_weft": {
            "height": options.height,
            "number_format": options.number_format,
            "y_unit": options.y_unit,
        },
    });

    if kind == "pie" {
        option["xAxis"] = Value::Null;
        option["yAxis"] = Value::Null;
        option["grid"] = Value::Null;
        option["legend"] = json!({"show": legend_show, "bottom": 0, "type": "scroll"});
        option["series"] = json!([{
            "type": "pie",
            "name": blueprint.y_fields[0],
            "radius": ["32%", "68%"],
            "center": ["50%", "48%"],
            "encode": {"itemName": blueprint.x_field, "value": blueprint.y_fields[0]},
            "label": {"show": labels, "formatter": "{b}: {c}"},
            "emphasis": {"itemStyle": {"shadowBlur": 10, "shadowColor": "rgba(0,0,0,.25)"}},
        }]);
        return Ok(option);
    }

    let horizontal = options.horizontal && kind == "bar";
    let value_axis_name = axis_name(&options.y_label, &options.y_unit);
    let category_axis_name = axis_name(&options.x_label, "");
    let x_axis = if horizontal {
        json!({"type": "value", "name": value_axis_name})
    } else if kind == "scatter" {
        json!({"type": axis_type_for(&blueprint.rows, &blueprint.x_field), "name": category_axis_name})
    } else {
        json!({"type": "category", "name": category_axis_name})
    };
    let mut y_axis = if horizontal {
        json!({"type": "category", "name": category_axis_name})
    } else {
        json!({
            "type": if options.log_y { "log" } else { "value" },
            "name": value_axis_name,
            "nameGap": 42,
        })
    };
    if !horizontal {
        if let Some(min) = options.y_min {
            y_axis["min"] = json!(min);
        }
        if let Some(max) = options.y_max {
            y_axis["max"] = json!(max);
        }
    }
    option["xAxis"] = x_axis;

    let dual_axis =
        options.dual_axis && !horizontal && kind != "scatter" && blueprint.y_fields.len() >= 2;
    if dual_axis {
        let mut right = y_axis.clone();
        right["position"] = json!("right");
        right["name"] = json!(axis_name(&options.y_label, &options.y_unit));
        option["yAxis"] = json!([y_axis, right]);
        grid["right"] = json!(64);
        option["grid"] = grid.clone();
    } else {
        option["yAxis"] = y_axis;
    }

    if options.data_zoom {
        grid["bottom"] = json!(if matches!(options.legend_position.as_str(), "bottom") {
            72
        } else {
            56
        });
        option["grid"] = grid.clone();
        option["dataZoom"] = json!([
            {"type": "inside"},
            {"type": "slider", "height": 18, "bottom": 0}
        ]);
    }

    let series = blueprint
        .y_fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let mut item = Map::new();
            item.insert("name".to_string(), Value::String(field.clone()));
            let series_type = match kind.as_str() {
                "line" | "area" => "line",
                "scatter" => "scatter",
                _ => "bar",
            };
            item.insert("type".to_string(), Value::String(series_type.to_string()));
            item.insert(
                "encode".to_string(),
                if horizontal {
                    json!({"x": field, "y": blueprint.x_field})
                } else {
                    json!({"x": blueprint.x_field, "y": field})
                },
            );
            if dual_axis && index > 0 {
                item.insert("yAxisIndex".to_string(), json!(1));
            }
            if kind == "area" {
                item.insert("areaStyle".to_string(), json!({"opacity": 0.22}));
                item.insert("smooth".to_string(), Value::Bool(options.smooth));
                item.insert("showSymbol".to_string(), Value::Bool(false));
            }
            if kind == "line" {
                item.insert("smooth".to_string(), Value::Bool(options.smooth));
                item.insert("showSymbol".to_string(), Value::Bool(false));
            }
            if options.stack && matches!(kind.as_str(), "bar" | "line" | "area") {
                item.insert("stack".to_string(), Value::String("total".to_string()));
            }
            if options.labels {
                item.insert("label".to_string(), json!({"show": true}));
            }
            if kind == "scatter" {
                item.insert("symbolSize".to_string(), json!(10));
            }
            Value::Object(item)
        })
        .collect::<Vec<_>>();
    option["series"] = Value::Array(series);
    Ok(option)
}

/// Create the `ChartParams` used by the model layer.
///
/// # Errors
/// Invalid visual data, SQL, or source configuration.
pub fn params_from_visual(
    title: &str,
    intent: &str,
    refresh_cron: &str,
    status: &str,
    input: VisualChartInput,
    allowed_tables: &[String],
) -> Result<chart_specs::ChartParams> {
    let source_type = match input.source_type.trim() {
        "" => "static".to_string(),
        other => other.to_string(),
    };
    let sql_query = input.sql_query.clone();
    let sql_limit = input.sql_limit;
    let http_url = input.http_url.clone();
    let http_data_path = input.http_data_path.clone();
    let blueprint = if source_type != "static" && input.rows.is_empty() {
        normalize_without_rows(
            input.columns,
            input.kind,
            input.x_field,
            input.y_fields,
            input.options,
        )?
    } else {
        normalize(input)?
    };
    let render_config = to_echarts(&blueprint)?;
    let meta = json!({
        "kind": blueprint.kind.clone(),
        "x_field": blueprint.x_field.clone(),
        "y_fields": blueprint.y_fields.clone(),
        "options": blueprint.options.clone(),
    });
    let data_source_config = match source_type.as_str() {
        "static" => json!({
            "data": blueprint.rows.clone(),
            "columns": blueprint.columns.clone(),
            "blueprint": meta,
        }),
        "sql" => {
            let query = sql_query.trim();
            if query.is_empty() {
                return Err(Error::string("SQL 查询不能为空"));
            }
            validate_readonly_sql(query, allowed_tables)?;
            json!({
                "query": query,
                "limit": sql_limit.unwrap_or(500).clamp(1, 2_000),
                "blueprint": meta,
            })
        }
        "http" => {
            let url = http_url.trim();
            if url.is_empty() {
                return Err(Error::string("HTTP 数据源 URL 不能为空"));
            }
            json!({
                "url": url,
                "data_path": http_data_path.trim(),
                "blueprint": meta,
            })
        }
        other => {
            return Err(Error::string(&format!(
                "unsupported chart data source `{other}`"
            )));
        }
    };
    Ok(chart_specs::ChartParams {
        title: title.trim().to_string(),
        intent: intent.to_string(),
        kind: blueprint.kind.clone(),
        data_source_type: source_type.clone(),
        data_source_config: data_source_config.to_string(),
        render_config: render_config.to_string(),
        refresh_cron: refresh_cron.to_string(),
        status: if status.trim().is_empty() {
            "active".to_string()
        } else {
            status.to_string()
        },
    })
}

/// Extract tabular rows and column names from a static/dynamic payload.
#[must_use]
pub fn rows_from_payload(payload: &Value) -> Vec<Value> {
    let array = if let Some(array) = payload.as_array() {
        array.clone()
    } else if let Some(rows) = payload.get("rows").and_then(Value::as_array) {
        rows.clone()
    } else if let Some(data) = payload.get("data").and_then(Value::as_array) {
        data.clone()
    } else {
        return Vec::new();
    };
    array.into_iter().take(MAX_ROWS).collect()
}

#[must_use]
pub fn columns_from_rows(rows: &[Value]) -> Vec<String> {
    let mut columns = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if seen.insert(key.clone()) {
                    columns.push(key.clone());
                }
            }
        }
    }
    columns
}

/// Derive editable visual state from a stored ECharts option. Used to open
/// legacy charts whose render_config was hand-written.
#[must_use]
pub fn derive_blueprint(
    title: &str,
    render_config: &Value,
    rows: Vec<Value>,
    columns: Vec<String>,
) -> ChartBlueprint {
    let columns = if columns.is_empty() {
        columns_from_rows(&rows)
    } else {
        columns
    };
    let mut kind = "bar".to_string();
    let mut x_field = String::new();
    let mut y_fields = Vec::new();
    let mut options = ChartOptions::default();
    if let Some(series) = render_config.get("series").and_then(Value::as_array) {
        for (index, item) in series.iter().enumerate() {
            let series_type = item.get("type").and_then(Value::as_str).unwrap_or("bar");
            if index == 0 {
                kind = match series_type {
                    "line" if item.get("areaStyle").is_some() => "area".to_string(),
                    "line" => "line".to_string(),
                    "pie" => "pie".to_string(),
                    "scatter" => "scatter".to_string(),
                    _ => "bar".to_string(),
                };
                if let Some(encode) = item.get("encode") {
                    x_field = encode
                        .get("x")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
            }
            if let Some(encode) = item.get("encode") {
                if let Some(y) = encode.get("y").and_then(Value::as_str) {
                    y_fields.push(y.to_string());
                } else if let Some(y) = encode.get("value").and_then(Value::as_str) {
                    y_fields.push(y.to_string());
                }
            }
        }
        if let Some(first) = series.first() {
            options.smooth = first
                .get("smooth")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            options.stack = first.get("stack").is_some();
            options.labels = first
                .get("label")
                .and_then(|label| label.get("show"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }
    }
    options.horizontal = render_config
        .get("xAxis")
        .and_then(|axis| axis.get("type"))
        .and_then(Value::as_str)
        == Some("value")
        && render_config
            .get("yAxis")
            .and_then(|axis| axis.get("type"))
            .and_then(Value::as_str)
            == Some("category");
    if let Some(show) = render_config
        .get("legend")
        .and_then(|legend| legend.get("show"))
        .and_then(Value::as_bool)
    {
        options.legend = show;
    }
    if x_field.is_empty() {
        x_field = infer_x_field(&rows, &columns);
    }
    if y_fields.is_empty() {
        y_fields = infer_y_fields(&rows, &columns, &x_field);
    }
    ChartBlueprint {
        title: title.to_string(),
        kind: normalize_kind(&kind),
        x_field,
        y_fields,
        options,
        columns,
        rows,
    }
}

/// Offline deterministic blueprint generation. It parses simple intent words
/// and otherwise produces a small, clearly labeled sample dataset.
#[must_use]
pub fn offline_blueprint(
    prompt: &str,
    parsed: Option<&ParsedData>,
    current: Option<&ChartBlueprint>,
) -> ChartBlueprint {
    let prompt_lower = prompt.to_ascii_lowercase();
    let kind = if ["饼", "pie", "环形"]
        .iter()
        .any(|word| prompt_lower.contains(word))
    {
        "pie"
    } else if ["散点", "scatter", "相关"]
        .iter()
        .any(|word| prompt_lower.contains(word))
    {
        "scatter"
    } else if ["面积", "area"]
        .iter()
        .any(|word| prompt_lower.contains(word))
    {
        "area"
    } else if ["折线", "趋势", "line", "时间"]
        .iter()
        .any(|word| prompt_lower.contains(word))
    {
        "line"
    } else {
        "bar"
    };
    let (columns, rows) = if let Some(parsed) = parsed.filter(|data| !data.rows.is_empty()) {
        (parsed.columns.clone(), parsed.rows.clone())
    } else if current.is_some_and(|current| !current.rows.is_empty()) {
        let current = current.expect("checked");
        (current.columns.clone(), current.rows.clone())
    } else {
        let columns = vec!["类别".to_string(), "数值".to_string()];
        let rows = vec![
            json!({"类别": "方案 A", "数值": 12}),
            json!({"类别": "方案 B", "数值": 19}),
            json!({"类别": "方案 C", "数值": 15}),
            json!({"类别": "方案 D", "数值": 24}),
            json!({"类别": "方案 E", "数值": 21}),
        ];
        (columns, rows)
    };
    let x_field = current
        .map(|current| current.x_field.clone())
        .filter(|field| columns.contains(field))
        .unwrap_or_else(|| infer_x_field(&rows, &columns));
    let mut y_fields = current
        .map(|current| current.y_fields.clone())
        .filter(|fields| fields.iter().any(|field| columns.contains(field)))
        .unwrap_or_default();
    let mentioned = columns
        .iter()
        .filter(|column| column.as_str() != x_field && prompt.contains(column.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !mentioned.is_empty() {
        y_fields = mentioned;
    }
    if y_fields.is_empty() {
        y_fields = infer_y_fields(&rows, &columns, &x_field);
    }
    let mut options = current
        .map(|current| current.options.clone())
        .unwrap_or_default();
    options.legend = true;
    if kind == "line" || kind == "area" {
        options.smooth = true;
    }
    if ["横向", "水平", "horizontal"]
        .iter()
        .any(|word| prompt_lower.contains(word))
    {
        options.horizontal = true;
    }
    ChartBlueprint {
        title: current
            .map(|current| current.title.clone())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| crate::util::truncate_chars(prompt.trim(), 60)),
        kind: kind.to_string(),
        x_field,
        y_fields,
        options,
        columns,
        rows,
    }
}

fn normalize_rows(columns: &mut Vec<String>, rows: Vec<Value>) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for row in rows.into_iter().take(MAX_ROWS) {
        let mut object = match row {
            Value::Object(object) => object,
            Value::Array(values) => {
                let mut object = Map::new();
                for (index, value) in values.into_iter().enumerate() {
                    let key = columns
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("列{}", index + 1));
                    object.insert(key, coerce_value(value));
                }
                object
            }
            value => {
                let key = columns.first().cloned().unwrap_or_else(|| "值".to_string());
                let mut object = Map::new();
                object.insert(key, coerce_value(value));
                object
            }
        };
        for value in object.values_mut() {
            *value = coerce_value(value.clone());
        }
        for key in object.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
        out.push(Value::Object(object));
    }
    if out.is_empty() {
        return Err(Error::string("数据为空"));
    }
    Ok(out)
}

fn coerce_value(value: Value) -> Value {
    let Value::String(text) = value else {
        return value;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Value::String(text);
    }
    let leading_zero = trimmed.len() > 1
        && trimmed.starts_with('0')
        && !trimmed.contains('.')
        && !trimmed.starts_with("0.");
    if !leading_zero {
        if let Ok(number) = trimmed.parse::<i64>() {
            return json!(number);
        }
        if let Ok(number) = trimmed.parse::<f64>() {
            if number.is_finite() {
                return json!(number);
            }
        }
    }
    if text.chars().count() > MAX_CELL_CHARS {
        return Value::String(crate::util::truncate_chars(&text, MAX_CELL_CHARS));
    }
    Value::String(text)
}

fn normalize_without_rows(
    columns: Vec<String>,
    kind: String,
    x_field: String,
    y_fields: Vec<String>,
    options: ChartOptions,
) -> Result<ChartBlueprint> {
    if columns.is_empty() {
        return Err(Error::string("请先选择或拉取数据列"));
    }
    let kind = normalize_kind(&kind);
    let options = normalize_options(options);
    let x_field = if x_field.is_empty() || !columns.contains(&x_field) {
        columns.first().cloned().unwrap_or_default()
    } else {
        x_field
    };
    let mut y_fields = y_fields
        .into_iter()
        .filter(|field| columns.contains(field) && field != &x_field)
        .collect::<Vec<_>>();
    if y_fields.is_empty() {
        y_fields = columns
            .iter()
            .filter(|column| **column != x_field)
            .cloned()
            .collect();
    }
    y_fields.truncate(MAX_Y_FIELDS);
    if y_fields.is_empty() {
        return Err(Error::string("请选择至少一个 Y 字段"));
    }
    let blueprint = ChartBlueprint {
        title: String::new(),
        kind,
        x_field,
        y_fields,
        options,
        columns,
        rows: Vec::new(),
    };
    to_echarts(&blueprint)?;
    Ok(blueprint)
}

fn normalize_options(mut options: ChartOptions) -> ChartOptions {
    options.palette = normalize_palette(&options.palette);
    options.sort = normalize_sort(&options.sort);
    options.height = if options.height.is_finite() {
        options.height.clamp(260.0, 900.0)
    } else {
        380.0
    };
    options.x_label = crate::util::truncate_chars(options.x_label.trim(), 60);
    options.y_label = crate::util::truncate_chars(options.y_label.trim(), 60);
    options.y_unit = crate::util::truncate_chars(options.y_unit.trim(), 20);
    options.number_format = match options.number_format.trim() {
        "integer" | "one_decimal" | "percent" | "compact" => {
            options.number_format.trim().to_string()
        }
        _ => "auto".to_string(),
    };
    options.legend_position = match options.legend_position.trim() {
        "bottom" | "right" => options.legend_position.trim().to_string(),
        _ => "top".to_string(),
    };
    options.y_min = options.y_min.filter(|value| value.is_finite());
    options.y_max = options.y_max.filter(|value| value.is_finite());
    if options.log_y && options.y_min.is_some_and(|value| value <= 0.0) {
        options.y_min = None;
    }
    if matches!((options.y_min, options.y_max), (Some(min), Some(max)) if min >= max) {
        options.y_min = None;
        options.y_max = None;
    }
    options
}

fn axis_name(label: &str, unit: &str) -> String {
    match (label.is_empty(), unit.is_empty()) {
        (true, true) => String::new(),
        (false, true) => label.to_string(),
        (true, false) => format!("（{unit}）"),
        (false, false) => format!("{label}（{unit}）"),
    }
}

fn normalize_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "line" => "line",
        "area" => "area",
        "pie" => "pie",
        "scatter" => "scatter",
        "column" | "bar" | "" => "bar",
        _ => "bar",
    }
    .to_string()
}

fn normalize_palette(palette: &str) -> String {
    match palette.trim() {
        "warm" | "cool" | "mono" => palette.trim().to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_sort(sort: &str) -> String {
    match sort.trim() {
        "asc" | "desc" => sort.trim().to_string(),
        _ => "none".to_string(),
    }
}

fn palette_colors(palette: &str) -> Value {
    match palette {
        "warm" => json!(["#a8442b", "#d98f70", "#e8c9a0", "#7b5e3b", "#c7a17a"]),
        "cool" => json!(["#2f6f8f", "#5aa9a3", "#8ec9c0", "#6b8eaf", "#c9e4de"]),
        "mono" => json!(["#3a332b", "#6f6a61", "#a49b8d", "#d8d1c7", "#8c8175"]),
        _ => json!(["#5470c6", "#91cc75", "#fac858", "#ee6666", "#73c0de", "#3ba272"]),
    }
}

fn infer_x_field(rows: &[Value], columns: &[String]) -> String {
    columns
        .iter()
        .find(|column| !is_numeric_column(rows, column))
        .cloned()
        .or_else(|| columns.first().cloned())
        .unwrap_or_default()
}

fn infer_y_fields(rows: &[Value], columns: &[String], x_field: &str) -> Vec<String> {
    let mut numeric = columns
        .iter()
        .filter(|column| column.as_str() != x_field && is_numeric_column(rows, column))
        .cloned()
        .collect::<Vec<_>>();
    if numeric.is_empty() {
        numeric = columns
            .iter()
            .filter(|column| column.as_str() != x_field)
            .cloned()
            .collect();
    }
    numeric.truncate(MAX_Y_FIELDS);
    numeric
}

fn is_numeric_column(rows: &[Value], column: &str) -> bool {
    let mut values = 0_usize;
    let mut numeric = 0_usize;
    for row in rows {
        let Some(value) = row.get(column) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        values += 1;
        if value.is_number()
            || value
                .as_str()
                .is_some_and(|text| text.parse::<f64>().is_ok())
        {
            numeric += 1;
        }
    }
    values > 0 && numeric * 2 >= values
}

fn axis_type_for(rows: &[Value], column: &str) -> &'static str {
    if is_numeric_column(rows, column) {
        "value"
    } else {
        "category"
    }
}

fn sort_rows(rows: &mut [Value], x_field: &str, y_fields: &[String], direction: &str) {
    let y = y_fields.first().cloned().unwrap_or_default();
    rows.sort_by(|left, right| {
        let left = sort_key(left, x_field, &y);
        let right = sort_key(right, x_field, &y);
        if direction == "desc" {
            right
                .partial_cmp(&left)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            left.partial_cmp(&right)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

fn sort_key(row: &Value, x_field: &str, y_field: &str) -> f64 {
    row.get(y_field)
        .or_else(|| row.get(x_field))
        .and_then(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            _ => None,
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VisualChartInput {
        VisualChartInput {
            kind: "line".to_string(),
            x_field: "月份".to_string(),
            y_fields: vec!["发布数".to_string()],
            columns: vec!["月份".to_string(), "发布数".to_string()],
            rows: vec![
                json!({"月份": "1月", "发布数": "3"}),
                json!({"月份": "2月", "发布数": 5}),
            ],
            ..VisualChartInput::default()
        }
    }

    #[test]
    fn visual_payload_becomes_echarts_option() {
        let blueprint = normalize(sample()).unwrap();
        assert_eq!(blueprint.kind, "line");
        assert_eq!(blueprint.rows[0]["发布数"], 3);
        let option = to_echarts(&blueprint).unwrap();
        assert_eq!(option["series"][0]["type"], "line");
        assert_eq!(option["dataset"]["source"], json!([]));
    }

    #[test]
    fn chart_details_are_applied_to_echarts() {
        let mut input = sample();
        input.options.height = 520.0;
        input.options.x_label = "月份".to_string();
        input.options.y_label = "发布数量".to_string();
        input.options.y_unit = "篇".to_string();
        input.options.number_format = "integer".to_string();
        input.options.y_min = Some(0.0);
        input.options.y_max = Some(10.0);
        input.options.data_zoom = true;
        input.options.legend_position = "bottom".to_string();
        let blueprint = normalize(input).unwrap();
        let option = to_echarts(&blueprint).unwrap();
        assert_eq!(option["_weft"]["height"], 520.0);
        assert_eq!(option["_weft"]["number_format"], "integer");
        assert_eq!(option["yAxis"]["name"], "发布数量（篇）");
        assert_eq!(option["yAxis"]["min"], 0.0);
        assert_eq!(option["yAxis"]["max"], 10.0);
        assert!(option["dataZoom"].is_array());
        assert_eq!(option["legend"]["bottom"], 0);
    }

    #[test]
    fn dual_axis_assigns_second_series_to_right_axis() {
        let mut input = sample();
        input.y_fields = vec!["发布数".to_string(), "阅读数".to_string()];
        input.columns = vec![
            "月份".to_string(),
            "发布数".to_string(),
            "阅读数".to_string(),
        ];
        input.rows = vec![
            json!({"月份": "1月", "发布数": 3, "阅读数": 120}),
            json!({"月份": "2月", "发布数": 5, "阅读数": 240}),
        ];
        input.options.dual_axis = true;
        let blueprint = normalize(input).unwrap();
        let option = to_echarts(&blueprint).unwrap();
        assert!(option["yAxis"].is_array());
        assert_eq!(option["yAxis"][1]["position"], "right");
        assert_eq!(option["series"][1]["yAxisIndex"], 1);
    }

    #[test]
    fn pie_and_invalid_payload_are_handled() {
        let mut input = sample();
        input.kind = "pie".to_string();
        let params =
            params_from_visual("图", "意图", "0 0 4 * * * *", "active", input, &[]).unwrap();
        let config: Value = serde_json::from_str(&params.data_source_config).unwrap();
        assert_eq!(config["blueprint"]["kind"], "pie");
        assert!(config["data"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn dynamic_chart_can_be_saved_before_preview() {
        let input = VisualChartInput {
            kind: "bar".to_string(),
            x_field: "label".to_string(),
            y_fields: vec!["value".to_string()],
            columns: vec!["label".to_string(), "value".to_string()],
            source_type: "sql".to_string(),
            sql_query: "SELECT status AS label, COUNT(*) AS value FROM posts GROUP BY status"
                .to_string(),
            ..VisualChartInput::default()
        };
        let params =
            params_from_visual("图", "", "", "active", input, &["posts".to_string()]).unwrap();
        let config: Value = serde_json::from_str(&params.data_source_config).unwrap();
        assert!(config["data"].is_null());
        assert_eq!(
            config["query"],
            "SELECT status AS label, COUNT(*) AS value FROM posts GROUP BY status"
        );
        assert_eq!(params.kind, "bar");
    }

    #[test]
    fn sorts_rows_by_selected_y() {
        let mut input = sample();
        input.options.sort = "desc".to_string();
        let normalized = normalize(input).unwrap();
        assert_eq!(normalized.rows[0]["发布数"], 5);
        assert_eq!(normalized.rows[1]["发布数"], 3);
    }

    #[test]
    fn visual_payload_without_rows_is_rejected() {
        let input = VisualChartInput {
            columns: vec!["x".to_string()],
            rows: vec![],
            ..VisualChartInput::default()
        };
        assert!(normalize(input).is_err());
    }
}
