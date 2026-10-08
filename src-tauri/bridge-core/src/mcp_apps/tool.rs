//! The `visualize` tool as the model sees it.
//!
//! The description is the render-or-not rubric, not a feature list: it is the
//! text the model reads at the moment it decides whether a turn needs a
//! visual, so the when, the when-not and the form-by-question table all live
//! here rather than in the system prompt. Any change to it must report its
//! eval delta (`testing/evals/visualize/`).

use serde_json::{json, Value};

use super::catalog;
use super::spec::COLORS;

pub const TOOL_NAME: &str = "visualize";
pub const SERVER_NAME: &str = "bridge";
pub const VIEW_URI: &str = "ui://bridge/visual";
pub const VIEW_MIME: &str = "text/html;profile=mcp-app";

const WHEN: &str = "Draw a visual inline in the chat: a chart, a diagram, or a document block such as research findings with their sources, a comparison, or a table. You send data; Bridge draws it in its own style, directly under your message.

WHEN TO CALL
1. The user asks to see, draw, chart, plot, diagram, visualize, or compare something visually.
2. The answer compares 4 or more quantities, shows change across 4 or more points in time, or shows a part-to-whole split, a correlation, a drop-off, or what moved a total.
3. A process, structure, or mechanism has 4 or more steps, branches, or levels that prose would make the reader re-read.
4. Research produced numbers or claims from more than one source: draw a findings block with its sources, plus one chart if there are numbers.

DO NOT CALL
1. The answer is one fact, one number, yes/no, a definition, or fits in two sentences or a markdown table of 4 rows or fewer.
2. The turn is coding, editing files, running commands, or reviewing a diff, unless the user asked for a visual.
3. You cannot say where the values came from and the user did not ask for an estimate. Never invent numbers to fill a chart.
4. The same data was already drawn in this conversation. Refer back to it. Redraw (redraw: true) only if the user asked to see it again or to change it.
5. The user asked for text, brevity, or \"just tell me\".
6. The visual would decorate the answer rather than carry it. Most turns need no visual.
At most one visual per turn unless the user asked for several.";

const GROUNDING: &str = "GROUNDING (required)
- Declare every source in \"sources\": {id, kind, ref, title?}. kind \"web\": ref is the exact URL you fetched. \"tool\": ref names the tool call that returned the values. \"file\": ref is the workspace path. \"user\": the user gave the numbers. \"computed\": ref is the formula and \"from\" lists the source ids it reads. \"estimate\": ref states the basis in a few words.
- Every chart, metric, table and compare block lists its sources in \"sourceIds\". Every findings item lists its own.
- Copy values exactly as the source states them. A value you had to estimate gets \"estimate\": true on its row or item and cites an estimate source; Bridge draws it dashed and labelled. Never present an estimate as a fact.";

const SHAPE: &str = "SHAPE
{\"version\": 1, \"title\": \"...\", \"subtitle\"?, \"layout\"?: \"stack\"|\"grid\", \"blocks\": [1 to 6 blocks], \"sources\"?: [...], \"followUps\"?: [up to 4 short questions the user may ask next], \"notes\"?: [...], \"redraw\"?: true}

chart block: {\"family\": \"chart\", \"form\": ..., \"sourceIds\": [...], \"vegaLite\": {\"mark\": ..., \"data\": {\"values\": [{field: value}, ...]}, \"encoding\": {\"x\": {\"field\", \"type\"}, \"y\": {...}, \"color\"?: {\"field\"}}}, \"colors\"?: {\"<series value>\": COLOR}}
  A Vega-Lite subset: inline data.values only; no transform, config, params, layer, concat or url. Pre-compute aggregates. Marks: bar for bar, stacked-bar, grouped-bar, proportion, waterfall and funnel; line; area; point for scatter; rect for heatmap. stacked-bar and grouped-bar need color (the series). proportion needs color (the parts) and x (their size). heatmap needs x, y and color (the measure). waterfall: x is the step, y the signed change, and running totals carry \"total\": true. funnel: one axis is the ordered steps, the other the count.

document block: {\"family\": \"document\", \"form\": ..., \"sourceIds\"?: [...], \"content\": {...}}
  metric {items: [{label, value, delta?, tone?: positive|negative|neutral, note?, estimate?}]}, always next to another block
  cards {items: [{title, body?, meta?, tags?}]}
  compare {criteria: [...], options: [{name, highlight?, summary?, values: {criterion: value}}]}
  table {columns: [{key, label, align?, format?: text|number|percent|currency|compact}], rows: [{key: value}]}, more than 4 rows
  callout {tone: info|warning|success|danger|neutral, title?, body}
  steps {items: [{title, body?}]}
  checklist {items: [{text, done}]}
  findings {items: [{text, sourceIds: [...], estimate?}]}
  pros-cons {pros: [...], cons: [...]}
  glossary {items: [{term, definition}]}

diagram block: {\"family\": \"diagram\", \"form\": \"grid\", \"graph\": {\"nodes\": [{id, label?, row, col, emphasis?: default|muted|active, marker?: none|checkpoint|tip|continues}], \"edges\": [{from, to, curve?, emphasis?}], \"caption\", \"ariaLabel\"}}
  Place nodes by row and col; keep labels to 18 characters. Achromatic; one \"active\" path at most, for the thing the reader should follow; \"muted\" for paths that are not the point.

AFTER THE CALL
On success, do not restate the visual's values in prose: say what matters and why. If the call is refused, fix every listed problem and call once more; if it is refused again, answer in text.";

/// The tool description, listing exactly the forms this build draws.
pub fn description() -> String {
    let colors = COLORS.join("|");
    let mut forms = String::from("PICK THE FORM BY THE QUESTION AND THE DATA'S SHAPE, NEVER BY HABIT\n");
    for family in catalog::FAMILIES {
        let available: Vec<_> = catalog::available()
            .filter(|info| info.family == *family)
            .collect();
        for info in available {
            forms.push_str(&format!("- {}/{}: {}\n", info.family, info.form, info.answers));
        }
    }
    forms.push_str("If two forms fit, use the one with fewer marks. Never a pie chart.");
    format!(
        "{WHEN}\n\n{forms}\n\n{GROUNDING}\n\n{}",
        SHAPE.replace("COLOR", &colors)
    )
}

/// The input schema. Deliberately flat: no `oneOf`, no `$ref`, so every
/// harness's schema sanitizer passes it through intact. The validator is
/// the real contract.
pub fn input_schema() -> Value {
    let forms: Vec<&str> = catalog::available().map(|info| info.form).collect();
    json!({
        "type": "object",
        "properties": {
            "version": {"type": "integer", "enum": [1]},
            "title": {"type": "string", "maxLength": 80, "description": "What the visual shows, in a few words."},
            "subtitle": {"type": "string", "maxLength": 160},
            "layout": {"type": "string", "enum": ["stack", "grid"]},
            "blocks": {
                "type": "array",
                "minItems": 1,
                "maxItems": 6,
                "items": {
                    "type": "object",
                    "properties": {
                        "family": {"type": "string", "enum": catalog::FAMILIES},
                        "form": {"type": "string", "enum": forms},
                        "title": {"type": "string", "maxLength": 80},
                        "sourceIds": {"type": "array", "items": {"type": "string"}},
                        "vegaLite": {"type": "object", "description": "chart blocks: mark, data.values, encoding"},
                        "colors": {"type": "object", "additionalProperties": {"type": "string", "enum": COLORS}},
                        "content": {"type": "object", "description": "document blocks"},
                        "graph": {"type": "object", "description": "diagram blocks"}
                    },
                    "required": ["family", "form"]
                }
            },
            "sources": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "kind": {"type": "string", "enum": ["web", "tool", "file", "user", "computed", "estimate"]},
                        "ref": {"type": "string"},
                        "title": {"type": "string"},
                        "from": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id", "kind", "ref"]
                }
            },
            "followUps": {"type": "array", "maxItems": 4, "items": {"type": "string", "maxLength": 80}},
            "notes": {"type": "array", "items": {"type": "string"}},
            "redraw": {"type": "boolean"}
        },
        "required": ["version", "title", "blocks"]
    })
}

/// The full `tools/list` entry, with the MCP Apps view reference.
pub fn definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "title": "Visualize",
        "description": description(),
        "inputSchema": input_schema(),
        "annotations": {
            "title": "Visualize",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        },
        "_meta": {
            "ui": {"resourceUri": VIEW_URI, "visibility": ["model", "app"]}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_description_lists_exactly_the_available_forms() {
        let text = description();
        for info in catalog::FORMS {
            let line = format!("- {}/{}:", info.family, info.form);
            assert_eq!(text.contains(&line), info.available, "{line}");
        }
    }

    #[test]
    fn the_tool_description_carries_the_when_and_when_not_rubric_and_the_form_table() {
        let text = description();
        for needle in [
            "WHEN TO CALL",
            "DO NOT CALL",
            "one fact, one number",
            "Never invent numbers",
            "redraw: true",
            "Most turns need no visual",
            "NEVER BY HABIT",
            "GROUNDING",
            "estimate",
        ] {
            assert!(text.contains(needle), "missing {needle:?}");
        }
        assert!(!text.contains('\u{2014}'), "no em dashes in model-facing text");
    }

    #[test]
    fn the_input_schema_uses_no_oneof() {
        let schema = serde_json::to_string(&input_schema()).unwrap();
        for keyword in ["oneOf", "anyOf", "allOf", "$ref", "$defs"] {
            assert!(!schema.contains(keyword), "{keyword}");
        }
    }

    #[test]
    fn tools_list_entry_has_ui_meta_and_read_only_annotations() {
        let tool = definition();
        assert_eq!(tool["name"], TOOL_NAME);
        assert_eq!(tool["_meta"]["ui"]["resourceUri"], VIEW_URI);
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
    }
}
