//! Every visual form `VisualSpec` names, and which ones this build can draw.
//!
//! One table, read by the validator (an unavailable form is refused with the
//! list of available ones), by the tool description (which lists only what
//! renders), and by the drift test against `testing/fixtures/visualize/forms.json`,
//! which the frontend reads too. Adding a renderer means flipping `available`
//! here and nowhere else.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormInfo {
    pub family: &'static str,
    pub form: &'static str,
    pub available: bool,
    /// The question this form answers, as the tool description states it.
    pub answers: &'static str,
}

const fn form(
    family: &'static str,
    form: &'static str,
    available: bool,
    answers: &'static str,
) -> FormInfo {
    FormInfo {
        family,
        form,
        available,
        answers,
    }
}

pub const FAMILIES: &[&str] = &["chart", "diagram", "map", "time", "math", "document"];

pub const FORMS: &[FormInfo] = &[
    form("chart", "bar", true, "compare categories (sorted)"),
    form(
        "chart",
        "stacked-bar",
        true,
        "totals split into at most 6 parts, per category",
    ),
    form(
        "chart",
        "grouped-bar",
        true,
        "two or more series side by side, per category",
    ),
    form("chart", "line", true, "change over time"),
    form(
        "chart",
        "area",
        true,
        "change over time where the volume matters",
    ),
    form(
        "chart",
        "scatter",
        true,
        "relationship between two measures",
    ),
    form(
        "chart",
        "heatmap",
        true,
        "a measure across two categorical axes",
    ),
    form(
        "chart",
        "proportion",
        true,
        "part of a whole, at most 6 parts (never a pie)",
    ),
    form("chart", "waterfall", true, "what moved a total"),
    form("chart", "funnel", true, "drop-off between ordered steps"),
    form("chart", "bubble", false, "three measures at once"),
    form("chart", "histogram", false, "distribution of one measure"),
    form("chart", "box", false, "distribution across groups"),
    form("chart", "treemap", false, "sizes in a hierarchy"),
    form("chart", "sunburst", false, "sizes in a deep hierarchy"),
    form("chart", "sankey", false, "flow between stages"),
    form("chart", "slope", false, "change between two points in time"),
    form(
        "chart",
        "dumbbell",
        false,
        "gap between two states per category",
    ),
    form("chart", "candlestick", false, "open, high, low, close"),
    form("chart", "sparkline-grid", false, "many small trends"),
    form(
        "diagram",
        "grid",
        true,
        "a mechanism: a fork, a gate, a fan-in, laid out by hand",
    ),
    form("diagram", "flow", false, "a process or decision"),
    form("diagram", "tree", false, "structure of a hierarchy"),
    form("diagram", "org", false, "who reports to whom"),
    form("diagram", "mindmap", false, "ideas around a center"),
    form("diagram", "sequence", false, "who calls whom, in order"),
    form("diagram", "state", false, "a lifecycle"),
    form("diagram", "er", false, "entities and relationships"),
    form("diagram", "dependency", false, "what depends on what"),
    form(
        "diagram",
        "architecture",
        false,
        "components and how they connect",
    ),
    form("diagram", "venn", false, "overlap between sets"),
    form("diagram", "quadrant", false, "items on two axes"),
    form("map", "choropleth", false, "a measure by country or state"),
    form("map", "points", false, "places on a map"),
    form("map", "hex", false, "a measure by state, equal-area tiles"),
    form("time", "timeline", false, "events in order"),
    form("time", "lanes", false, "parallel work over time"),
    form("time", "week", false, "a week's schedule"),
    form("time", "month", false, "a month's schedule"),
    form("math", "equation", false, "a formula"),
    form("math", "plot", false, "curves of functions"),
    form("math", "matrix", false, "a matrix"),
    form("math", "vectors", false, "vectors in a plane"),
    form(
        "document",
        "metric",
        true,
        "two to six headline numbers next to a chart or findings",
    ),
    form(
        "document",
        "cards",
        true,
        "a set of items, each with a title and a line",
    ),
    form(
        "document",
        "compare",
        true,
        "options side by side against criteria",
    ),
    form(
        "document",
        "table",
        true,
        "exact values the reader will look up (more than 4 rows)",
    ),
    form(
        "document",
        "callout",
        true,
        "one caveat or warning that must not be missed",
    ),
    form("document", "steps", true, "an ordered procedure"),
    form(
        "document",
        "checklist",
        true,
        "what is done and what is left",
    ),
    form(
        "document",
        "findings",
        true,
        "research conclusions, each with its sources",
    ),
    form(
        "document",
        "pros-cons",
        true,
        "the case for and against one option",
    ),
    form(
        "document",
        "glossary",
        true,
        "terms the reader needs defined",
    ),
    form("document", "gallery", false, "images from cited pages"),
];

pub fn lookup(family: &str, form: &str) -> Option<&'static FormInfo> {
    FORMS
        .iter()
        .find(|info| info.family == family && info.form == form)
}

pub fn available() -> impl Iterator<Item = &'static FormInfo> {
    FORMS.iter().filter(|info| info.available)
}

pub fn available_in(family: &str) -> Vec<&'static str> {
    available()
        .filter(|info| info.family == family)
        .map(|info| info.form)
        .collect()
}

/// The catalog as the frontend fixture stores it.
pub fn as_json() -> serde_json::Value {
    serde_json::Value::Array(
        FORMS
            .iter()
            .map(|info| {
                serde_json::json!({
                    "family": info.family,
                    "form": info.form,
                    "available": info.available,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_belongs_to_a_known_family_and_is_listed_once() {
        let mut seen = std::collections::BTreeSet::new();
        for info in FORMS {
            assert!(FAMILIES.contains(&info.family), "{info:?}");
            assert!(seen.insert((info.family, info.form)), "duplicate {info:?}");
        }
    }

    #[test]
    fn the_frontend_fixture_matches_this_catalog() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/visualize/forms.json");
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("forms.json exists"))
                .expect("forms.json parses");
        assert_eq!(
            stored,
            as_json(),
            "regenerate testing/fixtures/visualize/forms.json from mcp_apps::catalog::as_json()"
        );
    }
}
