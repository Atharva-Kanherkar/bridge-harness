//! Form-fit lint: a valid spec can still be the wrong picture.
//!
//! Runs only on specs that already validate. Each rule names the block, says
//! what is wrong with the form for this data, and says what to do instead,
//! because the model reads the refusal and retries once.

use std::collections::BTreeSet;

use serde_json::Value;

use super::spec::{chart_rows, SpecError};

pub const MIN_SCATTER_POINTS: usize = 8;
pub const MAX_SERIES: usize = 6;
pub const MAX_BARS: usize = 40;

/// How a field reads once its values are seen, when the spec did not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Quantitative,
    Temporal,
    Nominal,
}

fn looks_temporal(text: &str) -> bool {
    let text = text.trim();
    let digits = |part: &str, len: usize| part.len() == len && part.chars().all(|ch| ch.is_ascii_digit());
    let date = text.split(['T', ' ']).next().unwrap_or("");
    let parts: Vec<&str> = date.split('-').collect();
    match parts.as_slice() {
        [year] => digits(year, 4),
        [year, month] => digits(year, 4) && digits(month, 2),
        [year, month, day] => digits(year, 4) && digits(month, 2) && digits(day, 2),
        _ => false,
    }
}

/// The declared encoding type, or the one the values imply.
pub fn field_kind(block: &Value, channel: &str) -> Option<FieldKind> {
    let definition = block.pointer(&format!("/vegaLite/encoding/{channel}"))?;
    match definition.get("type").and_then(Value::as_str) {
        Some("quantitative") => return Some(FieldKind::Quantitative),
        Some("temporal") => return Some(FieldKind::Temporal),
        // Ordinal is ordered: for a line that is the point, so it reads as time.
        Some("ordinal") => return Some(FieldKind::Temporal),
        Some("nominal") => return Some(FieldKind::Nominal),
        _ => {}
    }
    if definition.get("aggregate").and_then(Value::as_str) == Some("count") {
        return Some(FieldKind::Quantitative);
    }
    let field = definition.get("field")?.as_str()?;
    let values: Vec<&Value> = chart_rows(block).iter().filter_map(|row| row.get(field)).filter(|value| !value.is_null()).collect();
    if values.is_empty() {
        return None;
    }
    if values.iter().all(|value| value.is_number()) {
        return Some(FieldKind::Quantitative);
    }
    if values.iter().all(|value| value.as_str().is_some_and(looks_temporal)) {
        return Some(FieldKind::Temporal);
    }
    Some(FieldKind::Nominal)
}

fn distinct(block: &Value, channel: &str) -> usize {
    let Some(field) = block
        .pointer(&format!("/vegaLite/encoding/{channel}/field"))
        .and_then(Value::as_str)
    else {
        return 0;
    };
    chart_rows(block)
        .iter()
        .filter_map(|row| row.get(field))
        .map(Value::to_string)
        .collect::<BTreeSet<_>>()
        .len()
}

/// Lint a valid spec. Empty means the forms fit.
pub fn lint(spec: &Value) -> Vec<SpecError> {
    let mut errors = Vec::new();
    let blocks = spec["blocks"].as_array().cloned().unwrap_or_default();
    for (at, block) in blocks.iter().enumerate() {
        let path = format!("blocks[{at}]");
        let form = block["form"].as_str().unwrap_or("");
        if block["family"] != "chart" {
            continue;
        }
        let rows = chart_rows(block).len();
        let minimum = if form == "proportion" { 2 } else { 3 };
        if rows < minimum {
            errors.push(SpecError::new(
                &path,
                format!("{rows} value{} is too few for a chart: say it in a sentence instead of calling visualize", if rows == 1 { "" } else { "s" }),
            ));
            continue;
        }
        if matches!(form, "line" | "area") && field_kind(block, "x") == Some(FieldKind::Nominal) {
            errors.push(SpecError::new(
                format!("{path}.vegaLite.encoding.x"),
                format!("a {form} over unordered categories implies an order that is not there: use form \"bar\", or set x.type \"ordinal\" if the order is real"),
            ));
        }
        // A quantitative colour is a measure (a heatmap's cells), not a set
        // of series, so only categorical colour counts against the limit.
        let categorical = form != "heatmap" && field_kind(block, "color") != Some(FieldKind::Quantitative);
        let series = if categorical { distinct(block, "color") } else { 0 };
        if series > MAX_SERIES {
            let what = if matches!(form, "proportion" | "stacked-bar") { "parts" } else { "series" };
            errors.push(SpecError::new(
                format!("{path}.vegaLite.encoding.color"),
                format!("{series} {what} is more than {MAX_SERIES}: keep the largest {} and group the rest as \"Other\"", MAX_SERIES - 1),
            ));
        }
        if form == "scatter" && rows < MIN_SCATTER_POINTS {
            errors.push(SpecError::new(
                &path,
                format!("{rows} points is too few for a scatter: use a bar or a table, or say it in a sentence"),
            ));
        }
        if matches!(form, "bar" | "stacked-bar" | "grouped-bar" | "funnel" | "waterfall") {
            let category = if field_kind(block, "x") == Some(FieldKind::Quantitative) { "y" } else { "x" };
            let bars = distinct(block, category);
            if bars > MAX_BARS {
                errors.push(SpecError::new(
                    format!("{path}.vegaLite.encoding.{category}"),
                    format!("{bars} bars is more than {MAX_BARS}: show the top {} and group the rest", MAX_BARS / 2),
                ));
            }
        }
        if form == "heatmap" && (distinct(block, "x") > MAX_BARS || distinct(block, "y") > 30) {
            errors.push(SpecError::new(
                &path,
                "a heatmap with more than 40 columns or 30 rows is unreadable: bucket the axes",
            ));
        }
    }

    if let [only] = blocks.as_slice() {
        let form = only["form"].as_str().unwrap_or("");
        if only["family"] == "document" && form == "metric" {
            errors.push(SpecError::new(
                "blocks[0]",
                "a metric block on its own is a sentence: say the numbers in text, or pair them with the chart or findings they summarise",
            ));
        }
        if only["family"] == "document" && form == "table" {
            let rows = only.pointer("/content/rows").and_then(Value::as_array).map_or(0, Vec::len);
            let columns = only.pointer("/content/columns").and_then(Value::as_array).map_or(0, Vec::len);
            if rows <= 4 && columns <= 3 {
                errors.push(SpecError::new(
                    "blocks[0]",
                    format!("a {rows}-row, {columns}-column table reads fine as a markdown table: answer in text instead"),
                ));
            }
        }
    }

    let chart_row_sets: Vec<BTreeSet<String>> = blocks
        .iter()
        .filter(|block| block["family"] == "chart")
        .map(|block| chart_rows(block).iter().map(super::canonical_json).collect())
        .collect();
    for (at, block) in blocks.iter().enumerate() {
        if block["family"] != "document" || block["form"] != "table" {
            continue;
        }
        let rows: BTreeSet<String> = block
            .pointer("/content/rows")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().map(super::canonical_json).collect())
            .unwrap_or_default();
        if !rows.is_empty() && chart_row_sets.iter().any(|chart| chart == &rows) {
            errors.push(SpecError::new(
                format!("blocks[{at}]"),
                "this table repeats the chart's rows, and the chart already shows each value on hover: drop the table",
            ));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chart(form: &str, mark: &str, rows: Vec<Value>, encoding: Value) -> Value {
        json!({
            "version": 1, "title": "t",
            "sources": [{"id": "s", "kind": "user", "ref": "entry-1"}],
            "blocks": [{"family": "chart", "form": form, "sourceIds": ["s"],
                "vegaLite": {"mark": mark, "data": {"values": rows}, "encoding": encoding}}]
        })
    }

    fn xy(x: &str, y: &str) -> Value {
        json!({"x": {"field": x}, "y": {"field": y}})
    }

    fn rows(count: usize) -> Vec<Value> {
        (0..count).map(|at| json!({"k": format!("k{at}"), "v": at})).collect()
    }

    #[test]
    fn fewer_than_three_values_says_use_a_sentence() {
        let errors = lint(&chart("bar", "bar", rows(2), xy("k", "v")));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("sentence"));
        let mut proportion = chart("proportion", "bar", rows(2), json!({"x": {"field": "v"}, "color": {"field": "k"}}));
        assert!(lint(&proportion).is_empty());
        proportion["blocks"][0]["vegaLite"]["data"]["values"] = json!(rows(1));
        assert_eq!(lint(&proportion).len(), 1);
    }

    #[test]
    fn line_over_unordered_categories_suggests_bar() {
        let errors = lint(&chart("line", "line", rows(5), xy("k", "v")));
        assert_eq!(errors[0].path, "blocks[0].vegaLite.encoding.x");
        assert!(errors[0].message.contains("\"bar\""));
        let years: Vec<Value> = (2015..2025).map(|year| json!({"k": year.to_string(), "v": year})).collect();
        assert!(lint(&chart("line", "line", years, xy("k", "v"))).is_empty());
        let mut ordinal = chart("line", "line", rows(5), xy("k", "v"));
        ordinal["blocks"][0]["vegaLite"]["encoding"]["x"]["type"] = json!("ordinal");
        assert!(lint(&ordinal).is_empty());
    }

    #[test]
    fn more_than_six_series_or_parts_is_rejected() {
        let data: Vec<Value> = (0..7).map(|at| json!({"k": "a", "s": format!("s{at}"), "v": 1})).chain((0..7).map(|at| json!({"k": "b", "s": format!("s{at}"), "v": 1}))).chain((0..7).map(|at| json!({"k": "c", "s": format!("s{at}"), "v": 1}))).collect();
        let errors = lint(&chart("stacked-bar", "bar", data, json!({"x": {"field": "k"}, "y": {"field": "v"}, "color": {"field": "s"}})));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].message.contains("7 parts"));
    }

    #[test]
    fn a_heatmaps_colour_is_a_measure_not_series() {
        let cells: Vec<Value> = (0..20).map(|at| json!({"a": format!("a{}", at % 5), "b": format!("b{}", at / 5), "n": at})).collect();
        let spec = chart("heatmap", "rect", cells, json!({"x": {"field": "a"}, "y": {"field": "b"}, "color": {"field": "n"}}));
        assert!(lint(&spec).is_empty());
    }

    #[test]
    fn scatter_with_fewer_than_8_points_is_rejected() {
        let points: Vec<Value> = (0..7).map(|at| json!({"a": at, "b": at * 2})).collect();
        assert_eq!(lint(&chart("scatter", "point", points, xy("a", "b"))).len(), 1);
        let points: Vec<Value> = (0..8).map(|at| json!({"a": at, "b": at * 2})).collect();
        assert!(lint(&chart("scatter", "point", points, xy("a", "b"))).is_empty());
    }

    #[test]
    fn more_than_40_bars_suggests_top_n() {
        let errors = lint(&chart("bar", "bar", rows(41), xy("k", "v")));
        assert_eq!(errors[0].path, "blocks[0].vegaLite.encoding.x");
        assert!(errors[0].message.contains("top 20"));
        let horizontal = chart("bar", "bar", rows(41), xy("v", "k"));
        assert_eq!(lint(&horizontal)[0].path, "blocks[0].vegaLite.encoding.y");
    }

    #[test]
    fn a_metric_block_alone_is_rejected() {
        let spec = json!({"version": 1, "title": "t", "blocks": [{"family": "document", "form": "metric", "content": {"items": [{"label": "a", "value": "1"}]}}]});
        assert_eq!(lint(&spec).len(), 1);
    }

    #[test]
    fn a_small_table_alone_suggests_markdown() {
        let table = |rows: usize| json!({"version": 1, "title": "t", "blocks": [{"family": "document", "form": "table", "content": {
            "columns": [{"key": "a", "label": "A"}],
            "rows": (0..rows).map(|at| json!({"a": at})).collect::<Vec<_>>()
        }}]});
        assert!(lint(&table(4))[0].message.contains("markdown"));
        assert!(lint(&table(5)).is_empty());
    }

    #[test]
    fn a_table_repeating_a_charts_rows_is_rejected() {
        let mut spec = chart("bar", "bar", rows(5), xy("k", "v"));
        spec["blocks"].as_array_mut().unwrap().push(json!({"family": "document", "form": "table", "content": {
            "columns": [{"key": "k", "label": "K"}, {"key": "v", "label": "V"}],
            "rows": rows(5)
        }}));
        let errors = lint(&spec);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "blocks[1]");
    }

    #[test]
    fn golden_specs_pass_validation_and_lint() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/visualize/specs.json");
        let fixtures: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("specs.json exists")).unwrap();
        for fixture in fixtures.iter().filter(|fixture| fixture["valid"] == json!(true)) {
            let errors = lint(&fixture["spec"]);
            assert!(errors.is_empty(), "{}: {errors:?}", fixture["name"]);
        }
    }

    #[test]
    fn lint_errors_carry_paths_and_a_suggested_fix() {
        for spec in [
            chart("bar", "bar", rows(2), xy("k", "v")),
            chart("line", "line", rows(5), xy("k", "v")),
            chart("bar", "bar", rows(41), xy("k", "v")),
        ] {
            for error in lint(&spec) {
                assert!(error.path.starts_with("blocks["), "{error:?}");
                assert!(error.message.contains(':'), "a fix follows the colon: {error:?}");
            }
        }
    }
}
