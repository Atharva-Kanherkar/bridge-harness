use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::*;
use crate::notes::doc::PropertyType;
use crate::notes::service::tests::service;
use crate::notes::service::{CreatePage, NewOption, NewProperty, NotesService, SchemaOp, UpdateNote, PERSON};

/// A small deterministic generator, so a failure reproduces.
pub(crate) struct Lcg(u64);

impl Lcg {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A database with one property of every type, and its ids by name.
pub(crate) struct Fixture {
    pub notes: NotesService,
    pub _dir: tempfile::TempDir,
    pub db: String,
    pub props: BTreeMap<&'static str, String>,
    pub rows: Vec<String>,
    pub books: Vec<String>,
}

impl Fixture {
    pub(crate) fn option(&self, prop: &str, name: &str) -> String {
        let database = self.notes.database(&self.db).unwrap();
        database.schema[&self.props[prop]].options.iter().find(|option| option.name == name).unwrap().id.clone()
    }

    pub(crate) fn doc(&self) -> DatabaseDoc {
        self.notes.database(&self.db).unwrap()
    }
}

fn options(names: &[(&str, Option<&str>)]) -> Vec<NewOption> {
    names.iter().map(|(name, group)| NewOption { name: (*name).into(), group: group.map(str::to_owned), ..Default::default() }).collect()
}

pub(crate) fn fixture(count: usize, seed: u64) -> Fixture {
    let (dir, notes) = service();
    let library = notes.create(CreatePage { title: Some("Library".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id;
    let books: Vec<String> = (0..4)
        .map(|index| notes.create(CreatePage { title: Some(format!("Shelf {index}")), parent_id: Some(library.clone()), ..Default::default() }, PERSON).unwrap().id)
        .collect();
    let db = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id;
    let add = |name: &str, kind: &str, opts: Vec<NewOption>, relation: Option<String>, date_format: Option<&str>| {
        SchemaOp::Add(NewProperty { name: name.into(), kind: kind.into(), options: opts, relation, date_format: date_format.map(str::to_owned) })
    };
    let added = notes
        .update_schema(
            &db,
            &[
                add("Note", "text", vec![], None, None),
                add("Link", "url", vec![], None, None),
                add("Rating", "number", vec![], None, None),
                add("Genre", "select", options(&[("Fiction", None), ("Tech", None), ("History", None)]), None, None),
                add("Status", "status", options(&[("To do", Some("todo")), ("Reading", Some("inProgress")), ("Paused", Some("inProgress")), ("Done", Some("done"))]), None, None),
                add("Labels", "multiSelect", options(&[("Short", None), ("Long", None), ("Gift", None)]), None, None),
                add("Due", "date", vec![], None, Some("date")),
                add("At", "date", vec![], None, Some("datetime")),
                add("Owned", "checkbox", vec![], None, None),
                add("Shelf", "relation", vec![], Some(library.clone()), None),
            ],
            PERSON,
        )
        .unwrap();
    let props: BTreeMap<&'static str, String> = ["Note", "Link", "Rating", "Genre", "Status", "Labels", "Due", "At", "Owned", "Shelf"]
        .into_iter()
        .map(|name| (name, added.added.iter().find(|(_, added)| added == name).unwrap().0.clone()))
        .collect();
    let mut fixture = Fixture { notes, _dir: dir, db, props, rows: Vec::new(), books };
    let opt = |fixture: &Fixture, prop: &str| -> Vec<String> { fixture.doc().schema[&fixture.props[prop]].options.iter().map(|option| option.id.clone()).collect() };
    let genres = opt(&fixture, "Genre");
    let statuses = opt(&fixture, "Status");
    let labels = opt(&fixture, "Labels");
    let words = ["alpha", "Beta", "gamma ray", "Delta", "", "alphabet", "ALPHA", "zeta"];
    let kinds = ["note", "idea", "task", "question"];
    let states = ["inbox", "open", "doing", "done", "archived"];
    let mut rng = Lcg(seed);
    for index in 0..count {
        let maybe = |rng: &mut Lcg| rng.below(4) != 0;
        let title = if rng.below(10) == 0 { None } else { Some(format!("{} {index:03}", words[rng.below(words.len() as u64) as usize])) };
        let row = fixture
            .notes
            .create(
                CreatePage {
                    title,
                    parent_id: Some(fixture.db.clone()),
                    kind: Some(kinds[rng.below(4) as usize].into()),
                    state: Some(states[rng.below(5) as usize].into()),
                    tags: if rng.below(3) == 0 { vec![["red", "blue", "green"][rng.below(3) as usize].into()] } else { vec![] },
                    workspace_id: (rng.below(3) == 0).then(|| "ws-known".to_owned()),
                    ..Default::default()
                },
                PERSON,
            )
            .unwrap()
            .id;
        if rng.below(4) == 0 {
            fixture.notes.update(UpdateNote { id: row.clone(), add_session_id: Some(format!("chat-{index}")), ..Default::default() }, PERSON).unwrap();
        }
        let mut values: Vec<(String, Option<Value>)> = Vec::new();
        let mut put = |name: &str, value: Value| values.push((fixture.props[name].clone(), Some(value)));
        if maybe(&mut rng) {
            put("Note", json!(words[rng.below(words.len() as u64) as usize]));
        }
        if maybe(&mut rng) {
            put("Link", json!(format!("https://{}.dev", ["a", "b", "c"][rng.below(3) as usize])));
        }
        if maybe(&mut rng) {
            put("Rating", json!((rng.below(11) as f64) / 2.0));
        }
        if maybe(&mut rng) {
            put("Genre", json!(genres[rng.below(3) as usize]));
        }
        if maybe(&mut rng) {
            put("Status", json!(statuses[rng.below(4) as usize]));
        }
        if maybe(&mut rng) {
            let mut picked: Vec<String> = labels.iter().filter(|_| rng.below(2) == 0).cloned().collect();
            picked.dedup();
            put("Labels", json!(picked));
        }
        if maybe(&mut rng) {
            let day = 1 + rng.below(28);
            let start = format!("2026-11-{day:02}");
            if rng.below(4) == 0 {
                put("Due", json!({"start": start, "end": format!("2026-11-{:02}", (day + 2).min(28))}));
            } else {
                put("Due", json!({"start": start}));
            }
        }
        if maybe(&mut rng) {
            put("At", json!({"start": format!("2026-11-{:02}T{:02}:30:00-08:00", 1 + rng.below(28), rng.below(24))}));
        }
        if maybe(&mut rng) {
            put("Owned", json!(rng.below(2) == 0));
        }
        if maybe(&mut rng) {
            let mut shelf: Vec<String> = fixture.books.iter().filter(|_| rng.below(3) == 0).cloned().collect();
            shelf.dedup();
            put("Shelf", json!(shelf));
        }
        if !values.is_empty() {
            fixture.notes.set_values(&row, &values, PERSON).unwrap();
        }
        fixture.rows.push(row);
    }
    fixture
}

// ---------------------------------------------------------------------------
// The reference: the same rules, evaluated directly over loaded rows.
// ---------------------------------------------------------------------------

fn lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

fn day_of_value(raw: &str) -> Option<NaiveDate> {
    database::instant(raw).map(|at| at.date_naive())
}

fn text_cell(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

/// Whether one row matches a filter, by the contract's rules.
pub(crate) fn reference(filter: &Value, row: &Row, database: &DatabaseDoc, today: NaiveDate) -> bool {
    if let Some(items) = filter.get("and").and_then(Value::as_array) {
        return items.iter().all(|item| reference(item, row, database, today));
    }
    if let Some(items) = filter.get("or").and_then(Value::as_array) {
        return items.is_empty() || items.iter().any(|item| reference(item, row, database, today));
    }
    let id = filter["property"].as_str().unwrap();
    let op = filter["op"].as_str().unwrap();
    let value = filter.get("value");
    let (positive, negate) = match op {
        "notContains" => ("contains", true),
        "notEquals" => ("equals", true),
        "ne" => ("eq", true),
        "isNot" => ("is", true),
        "isNoneOf" => ("isAnyOf", true),
        "isEmpty" => ("isNotEmpty", true),
        other => (other, false),
    };
    let note = &row.note;
    let cell = row.values.get(id);
    let text_test = |cell: Option<String>| -> bool {
        let Some(cell) = cell else { return positive == "isNotEmpty" && false };
        let needle = value.and_then(Value::as_str).map(lower).unwrap_or_default();
        match positive {
            "isNotEmpty" => true,
            "contains" => lower(&cell).contains(&needle),
            "equals" => lower(&cell) == needle,
            "startsWith" => lower(&cell).starts_with(&needle),
            _ => unreachable!("{positive}"),
        }
    };
    let day_test = |cell: Option<NaiveDate>| -> bool {
        let Some(cell) = cell else { return false };
        if positive == "isNotEmpty" {
            return true;
        }
        if positive == "within" {
            let (from, to) = within_range(value.unwrap().as_str().unwrap(), today).unwrap();
            return cell >= from && cell <= to;
        }
        let wanted = day_of_value(value.unwrap().as_str().unwrap()).unwrap();
        match positive {
            "before" => cell < wanted,
            "after" => cell > wanted,
            "onOrBefore" => cell <= wanted,
            "onOrAfter" => cell >= wanted,
            "equals" => cell == wanted,
            _ => unreachable!("{positive}"),
        }
    };
    let choice_test = |cell: Option<String>, groups: Option<&PropertyDef>| -> bool {
        let Some(cell) = cell.filter(|cell| !cell.is_empty()) else { return false };
        match positive {
            "isNotEmpty" => true,
            "is" => Some(cell.as_str()) == value.and_then(Value::as_str),
            "isAnyOf" => value.unwrap().as_array().unwrap().iter().any(|item| item.as_str() == Some(cell.as_str())),
            "inGroup" => groups.and_then(|def| def.option(&cell)).and_then(|option| option.group.as_deref()) == value.and_then(Value::as_str),
            _ => unreachable!("{positive}"),
        }
    };
    let list_test = |cell: Vec<String>| -> bool {
        match positive {
            "isNotEmpty" => !cell.is_empty(),
            "contains" => cell.iter().any(|item| Some(item.as_str()) == value.and_then(Value::as_str)),
            "containsAny" => value.unwrap().as_array().unwrap().iter().any(|wanted| cell.iter().any(|item| Some(item.as_str()) == wanted.as_str())),
            _ => unreachable!("{positive}"),
        }
    };
    let list_of = |cell: Option<&Value>| -> Vec<String> {
        cell.and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
    };
    let result = if let Some(def) = database.schema.get(id) {
        match def.kind {
            PropertyType::Text | PropertyType::Url => text_test(text_cell(cell.and_then(Value::as_str))),
            PropertyType::Number => {
                let Some(number) = cell.and_then(Value::as_f64) else { return negate };
                let wanted = value.and_then(Value::as_f64).unwrap_or_default();
                match positive {
                    "isNotEmpty" => true,
                    "eq" => number == wanted,
                    "lt" => number < wanted,
                    "lte" => number <= wanted,
                    "gt" => number > wanted,
                    "gte" => number >= wanted,
                    _ => unreachable!(),
                }
            }
            PropertyType::Date => day_test(cell.and_then(|cell| cell.get("start")).and_then(Value::as_str).and_then(day_of_value)),
            PropertyType::Select | PropertyType::Status => choice_test(cell.and_then(Value::as_str).map(str::to_owned), Some(def)),
            PropertyType::MultiSelect | PropertyType::Relation => list_test(list_of(cell)),
            PropertyType::Checkbox => cell.and_then(Value::as_bool).unwrap_or(false) == value.and_then(Value::as_bool).unwrap(),
        }
    } else {
        match id {
            "title" => text_test(text_cell(note.title.as_deref())),
            "createdBy" => text_test(text_cell(Some(note.created_by.as_str()))),
            "kind" => choice_test(Some(note.kind.clone()), None),
            "state" => choice_test(Some(note.state.clone()), None),
            "project" => choice_test(note.workspace_id.clone(), None),
            "tags" => list_test(note.tags.clone()),
            "chats" => !note.session_ids.is_empty(),
            "parent" => match positive {
                "isNotEmpty" => note.parent_id.is_some(),
                "equals" => note.parent_id.as_deref() == value.and_then(Value::as_str),
                _ => unreachable!(),
            },
            "created" => day_test(day_of_value(&note.created_at)),
            "updated" => day_test(day_of_value(&note.updated_at)),
            other => panic!("unknown property {other}"),
        }
    };
    result != negate
}

fn all_rows(fixture: &Fixture) -> Vec<Row> {
    let ids: Vec<&str> = fixture.rows.iter().map(String::as_str).collect();
    fixture.notes.with_db(|db| super::load_rows(db, &ids)).unwrap().into_values().collect()
}

fn ids_of(rows: &[Row]) -> BTreeSet<String> {
    rows.iter().map(|row| row.note.id.clone()).collect()
}

fn query(fixture: &Fixture, spec: QuerySpec) -> QueryResult {
    fixture.notes.query_database(&fixture.db, &QuerySpec { limit: Some(MAX_LIMIT), ..spec }).unwrap()
}

fn filtered(fixture: &Fixture, filter: Value, today: NaiveDate) -> BTreeSet<String> {
    let result = query(fixture, QuerySpec { filter: Some(filter), today: Some(today.format("%Y-%m-%d").to_string()), ..Default::default() });
    assert_eq!(result.rows.len(), result.total, "every row fits one page here");
    ids_of(&result.rows)
}

#[test]
fn every_op_matches_a_reference_filter() {
    let fixture = fixture(300, 42);
    let database = fixture.doc();
    let rows = all_rows(&fixture);
    let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
    let p = |name: &str| fixture.props[name].clone();
    let genre = |name: &str| fixture.option("Genre", name);
    let status = |name: &str| fixture.option("Status", name);
    let label = |name: &str| fixture.option("Labels", name);
    let mut cases: Vec<Value> = Vec::new();
    let cond = |property: &str, op: &str, value: Option<Value>| {
        let mut condition = json!({"property": property, "op": op});
        if let Some(value) = value {
            condition["value"] = value;
        }
        condition
    };
    for (property, needle) in [(p("Note"), "alpha"), (p("Link"), "https://b"), ("title".to_owned(), "ALPHA"), ("createdBy".to_owned(), "pers")] {
        for op in ["contains", "notContains", "equals", "notEquals", "startsWith"] {
            cases.push(cond(&property, op, Some(json!(needle))));
        }
        cases.push(cond(&property, "equals", Some(json!("beta"))));
        cases.push(cond(&property, "contains", Some(json!(""))));
        cases.push(cond(&property, "isEmpty", None));
        cases.push(cond(&property, "isNotEmpty", None));
    }
    for op in ["eq", "ne", "lt", "lte", "gt", "gte"] {
        for value in [0.0, 2.5, 5.0] {
            cases.push(cond(&p("Rating"), op, Some(json!(value))));
        }
    }
    cases.push(cond(&p("Rating"), "isEmpty", None));
    cases.push(cond(&p("Rating"), "isNotEmpty", None));
    for property in [p("Due"), p("At"), "created".to_owned(), "updated".to_owned()] {
        for op in ["before", "after", "onOrBefore", "onOrAfter", "equals"] {
            for value in ["2026-11-10", "2026-11-10T03:00:00-08:00"] {
                cases.push(cond(&property, op, Some(json!(value))));
            }
        }
        for window in WITHIN {
            cases.push(cond(&property, "within", Some(json!(window))));
        }
        cases.push(cond(&property, "isEmpty", None));
        cases.push(cond(&property, "isNotEmpty", None));
    }
    for (property, values) in [
        (p("Genre"), vec![genre("Fiction"), genre("Tech")]),
        (p("Status"), vec![status("Reading"), status("Done")]),
        ("kind".to_owned(), vec!["task".to_owned(), "idea".to_owned()]),
        ("state".to_owned(), vec!["inbox".to_owned(), "archived".to_owned()]),
        ("project".to_owned(), vec!["ws-known".to_owned(), "ws-other".to_owned()]),
    ] {
        cases.push(cond(&property, "is", Some(json!(values[0]))));
        cases.push(cond(&property, "isNot", Some(json!(values[0]))));
        cases.push(cond(&property, "isAnyOf", Some(json!(values))));
        cases.push(cond(&property, "isNoneOf", Some(json!(values))));
        cases.push(cond(&property, "isAnyOf", Some(json!([]))));
        cases.push(cond(&property, "isEmpty", None));
        cases.push(cond(&property, "isNotEmpty", None));
    }
    for group in ["todo", "inProgress", "done"] {
        cases.push(cond(&p("Status"), "inGroup", Some(json!(group))));
    }
    for (property, values) in [
        (p("Labels"), vec![label("Short"), label("Gift")]),
        (p("Shelf"), vec![fixture.books[0].clone(), fixture.books[2].clone()]),
        ("tags".to_owned(), vec!["red".to_owned(), "green".to_owned()]),
    ] {
        cases.push(cond(&property, "contains", Some(json!(values[0]))));
        cases.push(cond(&property, "notContains", Some(json!(values[0]))));
        cases.push(cond(&property, "containsAny", Some(json!(values))));
        cases.push(cond(&property, "isEmpty", None));
        cases.push(cond(&property, "isNotEmpty", None));
    }
    cases.push(cond(&p("Owned"), "is", Some(json!(true))));
    cases.push(cond(&p("Owned"), "is", Some(json!(false))));
    cases.push(cond("chats", "isEmpty", None));
    cases.push(cond("chats", "isNotEmpty", None));
    cases.push(cond("parent", "isEmpty", None));
    cases.push(cond("parent", "isNotEmpty", None));
    cases.push(cond("parent", "equals", Some(json!(fixture.db))));
    cases.push(cond("parent", "notEquals", Some(json!(fixture.db))));
    // Nested groups, both ways round, and the empty group.
    cases.push(json!({"and": [cond(&p("Rating"), "gte", Some(json!(3))), {"or": [cond(&p("Genre"), "is", Some(json!(genre("Tech")))), cond(&p("Owned"), "is", Some(json!(true)))]}]}));
    cases.push(json!({"or": [cond("kind", "is", Some(json!("task"))), {"and": [cond(&p("Labels"), "isEmpty", None), cond(&p("Due"), "within", Some(json!("thisMonth")))]}]}));
    cases.push(json!({"and": []}));
    cases.push(json!({"or": []}));

    let mut nonempty = 0;
    for case in &cases {
        let expected: BTreeSet<String> = rows.iter().filter(|row| reference(case, row, &database, today)).map(|row| row.note.id.clone()).collect();
        if !expected.is_empty() && expected.len() < rows.len() {
            nonempty += 1;
        }
        assert_eq!(filtered(&fixture, case.clone(), today), expected, "the planner and the reference disagree on {case}");
    }
    assert!(nonempty * 2 > cases.len(), "most cases split the fixture, so they test something ({nonempty} of {})", cases.len());
}

#[test]
fn a_bad_filter_names_the_valid_choices() {
    let fixture = fixture(3, 1);
    let run = |filter: Value| fixture.notes.query_database(&fixture.db, &QuerySpec { filter: Some(filter), ..Default::default() }).unwrap_err().to_string();
    let error = run(json!({"property": "Ratings", "op": "eq", "value": 1}));
    assert!(error.contains("Rating (") && error.contains("title"), "{error}");
    let error = run(json!({"property": fixture.props["Rating"], "op": "contains", "value": 1}));
    assert!(error.contains("eq, ne, lt"), "{error}");
    let error = run(json!({"property": fixture.props["Rating"], "op": "eq", "value": "three"}));
    assert!(error.contains("takes a number"), "{error}");
    let error = run(json!({"property": fixture.props["Genre"], "op": "is", "value": "Fiction"}));
    assert!(error.contains("choose one of Fiction ("), "an option is named by id; the error lists both: {error}");
    let condition = json!({"property": "title", "op": "isEmpty"});
    let error = run(json!({"and": [{"or": [{"and": [condition]}]}]}));
    assert!(error.contains("two levels"), "{error}");
    let error = run(json!({"and": vec![condition; MAX_CONDITIONS + 1]}));
    assert!(error.contains("at most 20"), "{error}");
    let error = run(json!({"property": "title", "op": "isEmpty", "extra": 1}));
    assert!(error.contains("not extra"), "{error}");
    let error = fixture.notes.query_database(&fixture.db, &QuerySpec { sorts: Some(vec![SortDef { property: fixture.props["Labels"].clone(), direction: "asc".into() }]), ..Default::default() }).unwrap_err().to_string();
    assert!(error.contains("cannot be sorted"), "{error}");
}

#[test]
fn values_are_bound_never_interpolated() {
    let fixture = fixture(5, 2);
    let error = fixture
        .notes
        .query_database(&fixture.db, &QuerySpec { filter: Some(json!({"property": "'; DROP TABLE pages; --", "op": "isEmpty"})), ..Default::default() })
        .unwrap_err()
        .to_string();
    assert!(error.contains("no property"), "{error}");
    let row = fixture.rows[0].clone();
    fixture.notes.set_values(&row, &[(fixture.props["Note"].clone(), Some(json!("x' OR 1=1 --")))], PERSON).unwrap();
    let hits = filtered(&fixture, json!({"property": fixture.props["Note"], "op": "equals", "value": "' OR 1=1 --"}), chrono::Utc::now().date_naive());
    assert!(hits.is_empty(), "a quote is a character, not SQL");
    let hits = filtered(&fixture, json!({"property": fixture.props["Note"], "op": "contains", "value": "' OR 1=1 --"}), chrono::Utc::now().date_naive());
    assert_eq!(hits, BTreeSet::from([row]));
    let pages: i64 = fixture.notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))?)).unwrap();
    assert!(pages > 5);
}

#[test]
fn every_condition_uses_an_index() {
    let fixture = fixture(20, 3);
    let database = fixture.doc();
    let p = |name: &str| fixture.props[name].clone();
    let filter = json!({"and": [
        {"property": p("Note"), "op": "contains", "value": "a"},
        {"property": p("Rating"), "op": "gte", "value": 2},
        {"property": p("Due"), "op": "before", "value": "2026-11-20"},
        {"property": p("Genre"), "op": "isNot", "value": fixture.option("Genre", "Tech")},
        {"property": p("Labels"), "op": "containsAny", "value": [fixture.option("Labels", "Gift")]},
        {"property": p("Owned"), "op": "is", "value": true},
        {"property": p("Shelf"), "op": "isEmpty"},
    ]});
    let spec = QuerySpec {
        filter: Some(filter),
        sorts: Some(vec![SortDef { property: p("Rating"), direction: "desc".into() }, SortDef { property: p("Status"), direction: "asc".into() }]),
        group_by: Some(GroupByDef { property: p("Labels"), date_bucket: None }),
        ..Default::default()
    };
    let plan = fixture.notes.with_db(|db| super::explain(db, fixture.notes.space(), &database, &spec)).unwrap();
    let touching: Vec<&String> = plan.iter().filter(|line| line.contains("page_values")).collect();
    assert!(!touching.is_empty(), "{plan:#?}");
    for line in &touching {
        assert!(line.contains("USING INDEX") || line.contains("USING COVERING INDEX") || line.contains("USING PRIMARY KEY"), "page_values reached without an index: {line}\n{plan:#?}");
        assert!(!line.starts_with("SCAN page_values"), "{line}");
    }
    assert!(plan.iter().any(|line| line.contains("idx_page_values_num")), "the number condition uses its value index: {plan:#?}");
}

#[test]
fn sorts_are_total_and_empty_values_sort_last() {
    let fixture = fixture(60, 4);
    let p = |name: &str| fixture.props[name].clone();
    let database = fixture.doc();
    let rows = all_rows(&fixture);
    let by_id: BTreeMap<&str, &Row> = rows.iter().map(|row| (row.note.id.as_str(), row)).collect();
    for direction in ["asc", "desc"] {
        let result = query(&fixture, QuerySpec { sorts: Some(vec![SortDef { property: p("Rating"), direction: direction.into() }]), ..Default::default() });
        let ratings: Vec<Option<f64>> = result.rows.iter().map(|row| row.values.get(&p("Rating")).and_then(Value::as_f64)).collect();
        let present: Vec<f64> = ratings.iter().flatten().copied().collect();
        let mut sorted = present.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if direction == "desc" {
            sorted.reverse();
        }
        assert_eq!(present, sorted, "{direction}");
        let first_empty = ratings.iter().position(Option::is_none).unwrap_or(ratings.len());
        assert!(ratings[first_empty..].iter().all(Option::is_none), "empty ratings sort last when {direction}");
        // Equal keys fall back to updated desc, then id desc.
        for pair in result.rows.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if a.values.get(&p("Rating")) == b.values.get(&p("Rating")) {
                assert!((a.note.updated_at.as_str(), a.note.id.as_str()) > (b.note.updated_at.as_str(), b.note.id.as_str()));
            }
        }
    }
    // A status sorts by option order, not by its id or name.
    let result = query(&fixture, QuerySpec { sorts: Some(vec![SortDef { property: p("Status"), direction: "asc".into() }]), ..Default::default() });
    let order: Vec<usize> = database.schema[&p("Status")].options.iter().enumerate().map(|(index, _)| index).collect();
    let positions: Vec<usize> = result
        .rows
        .iter()
        .filter_map(|row| row.values.get(&p("Status")).and_then(Value::as_str))
        .map(|id| database.schema[&p("Status")].options.iter().position(|option| option.id == id).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] <= pair[1]), "{positions:?} in {order:?}");
    // A title sort folds ASCII case and puts untitled pages last.
    let result = query(&fixture, QuerySpec { sorts: Some(vec![SortDef { property: "title".into(), direction: "asc".into() }]), ..Default::default() });
    let titles: Vec<Option<String>> = result.rows.iter().map(|row| by_id[row.note.id.as_str()].note.title.clone().filter(|title| !title.is_empty())).collect();
    let present: Vec<String> = titles.iter().flatten().map(|title| title.to_ascii_lowercase()).collect();
    assert!(present.windows(2).all(|pair| pair[0] <= pair[1]), "{present:?}");
    let first_empty = titles.iter().position(Option::is_none).unwrap_or(titles.len());
    assert!(titles[first_empty..].iter().all(Option::is_none));
}

#[test]
fn a_cursor_neither_skips_nor_repeats() {
    let fixture = fixture(37, 5);
    let p = |name: &str| fixture.props[name].clone();
    let sorts = Some(vec![SortDef { property: p("Rating"), direction: "asc".into() }, SortDef { property: "title".into(), direction: "desc".into() }]);
    let page = |cursor: Option<String>| fixture.notes.query_database(&fixture.db, &QuerySpec { sorts: sorts.clone(), limit: Some(10), cursor, ..Default::default() }).unwrap();
    let full = fixture.notes.query_database(&fixture.db, &QuerySpec { sorts: sorts.clone(), limit: Some(500), ..Default::default() }).unwrap();
    assert_eq!(full.total, 37);

    let first = page(None);
    assert_eq!(first.rows.len(), 10);
    assert_eq!(first.total, 37);
    // Between reads: delete a row not yet seen, and add one that sorts after
    // the cursor and one that sorts before it.
    let seen: Vec<String> = first.rows.iter().map(|row| row.note.id.clone()).collect();
    let unseen = full.rows.iter().map(|row| row.note.id.clone()).find(|id| !seen.contains(id) && full.rows.iter().position(|row| &row.note.id == id).unwrap() > 20).unwrap();
    fixture.notes.delete(&unseen, PERSON).unwrap();
    let late = fixture.notes.create(CreatePage { title: Some("zz late".into()), parent_id: Some(fixture.db.clone()), ..Default::default() }, PERSON).unwrap().id;
    fixture.notes.set_values(&late, &[(p("Rating"), Some(json!(100)))], PERSON).unwrap();
    let early = fixture.notes.create(CreatePage { title: Some("aa early".into()), parent_id: Some(fixture.db.clone()), ..Default::default() }, PERSON).unwrap().id;
    fixture.notes.set_values(&early, &[(p("Rating"), Some(json!(-1)))], PERSON).unwrap();

    let mut collected = seen.clone();
    let mut cursor = first.next_cursor.clone();
    while let Some(next) = cursor {
        let result = page(Some(next));
        collected.extend(result.rows.iter().map(|row| row.note.id.clone()));
        cursor = result.next_cursor;
    }
    let unique: BTreeSet<&String> = collected.iter().collect();
    assert_eq!(unique.len(), collected.len(), "no row is shown twice");
    assert!(collected.contains(&late), "a row inserted after the cursor is reached");
    assert!(!collected.contains(&early), "a row inserted before the cursor stays behind it");
    assert!(!collected.contains(&unseen), "a deleted row is gone");
    let survivors: BTreeSet<String> = full.rows.iter().map(|row| row.note.id.clone()).filter(|id| id != &unseen).collect();
    assert!(survivors.iter().all(|id| collected.contains(id)), "every surviving row is seen exactly once");
    // The order across pages is the one-page order.
    let now = fixture.notes.query_database(&fixture.db, &QuerySpec { sorts, limit: Some(500), ..Default::default() }).unwrap();
    let expected: Vec<String> = now.rows.iter().map(|row| row.note.id.clone()).filter(|id| id != &early).collect();
    assert_eq!(collected, expected);
    // A cursor from another query shape is refused.
    let error = fixture.notes.query_database(&fixture.db, &QuerySpec { cursor: first.next_cursor, ..Default::default() }).unwrap_err().to_string();
    assert!(error.contains("cursor"), "{error}");
}

#[test]
fn groups_follow_option_order_and_multi_select_rows_repeat() {
    let fixture = fixture(80, 6);
    let p = |name: &str| fixture.props[name].clone();
    let database = fixture.doc();
    let rows = all_rows(&fixture);
    let group = |property: &str, bucket: Option<&str>, limit: Option<usize>| {
        query(&fixture, QuerySpec { group_by: Some(GroupByDef { property: property.into(), date_bucket: bucket.map(str::to_owned) }), group_limit: limit, ..Default::default() }).groups.unwrap()
    };
    // Status: every option in order, then No value, counts from the rows.
    let groups = group(&p("Status"), None, None);
    let keys: Vec<String> = groups.iter().map(|group| group.key.clone()).collect();
    let mut expected: Vec<String> = database.schema[&p("Status")].options.iter().map(|option| option.id.clone()).collect();
    expected.push(String::new());
    assert_eq!(keys, expected);
    for group in &groups {
        let count = rows.iter().filter(|row| row.values.get(&p("Status")).and_then(Value::as_str).unwrap_or_default() == group.key).count();
        assert_eq!(group.count, count, "{}", group.label);
        assert_eq!(group.rows.len(), count.min(DEFAULT_GROUP_LIMIT));
    }
    assert_eq!(groups.last().unwrap().label, "No value");
    // Labels: a row with two labels is in both groups.
    let groups = group(&p("Labels"), None, Some(2));
    let total: usize = groups.iter().map(|group| group.count).sum();
    let expected_total: usize = rows
        .iter()
        .map(|row| row.values.get(&p("Labels")).and_then(Value::as_array).map(|items| items.len().max(1)).unwrap_or(1))
        .sum();
    assert_eq!(total, expected_total);
    assert!(groups.iter().all(|group| group.rows.len() <= 2), "groupLimit caps each group's rows");
    // Checkbox: checked then unchecked, a missing cell counting as unchecked.
    let groups = group(&p("Owned"), None, None);
    assert_eq!(groups.iter().map(|group| group.key.as_str()).collect::<Vec<_>>(), vec!["true", "false"]);
    assert_eq!(groups.iter().map(|group| group.count).sum::<usize>(), rows.len());
    // Dates: the occurring buckets, ascending, then No value.
    for bucket in ["day", "week", "month"] {
        let groups = group(&p("Due"), Some(bucket), None);
        let keys: Vec<&str> = groups.iter().map(|group| group.key.as_str()).filter(|key| !key.is_empty()).collect();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{bucket}: {keys:?}");
        assert_eq!(groups.iter().map(|group| group.count).sum::<usize>(), rows.len(), "{bucket}");
        if bucket == "week" {
            for key in keys {
                assert_eq!(NaiveDate::parse_from_str(key, "%Y-%m-%d").unwrap().weekday(), chrono::Weekday::Mon);
            }
        }
    }
    // Kind: its fixed values in order.
    let groups = group("kind", None, None);
    assert_eq!(groups.iter().map(|group| group.key.as_str()).collect::<Vec<_>>(), vec!["note", "idea", "task", "question", "database", ""]);
    // A text property cannot be grouped.
    let error = fixture.notes.query_database(&fixture.db, &QuerySpec { group_by: Some(GroupByDef { property: p("Note"), date_bucket: None }), ..Default::default() }).unwrap_err().to_string();
    assert!(error.contains("cannot be grouped"), "{error}");
}

#[test]
fn within_uses_the_callers_today() {
    let fixture = fixture(0, 7);
    let p = |name: &str| fixture.props[name].clone();
    let mut rows = Vec::new();
    for (index, day) in ["2026-11-01", "2026-11-09", "2026-11-15", "2026-11-16", "2026-12-01"].iter().enumerate() {
        let row = fixture.notes.create(CreatePage { title: Some(format!("Day {index}")), parent_id: Some(fixture.db.clone()), ..Default::default() }, PERSON).unwrap().id;
        fixture.notes.set_values(&row, &[(p("Due"), Some(json!({"start": day})))], PERSON).unwrap();
        rows.push(row);
    }
    let within = |window: &str, today: &str| -> BTreeSet<String> {
        let result = query(&fixture, QuerySpec { filter: Some(json!({"property": p("Due"), "op": "within", "value": window})), today: Some(today.into()), ..Default::default() });
        ids_of(&result.rows)
    };
    // 2026-11-15 is a Sunday: its ISO week runs from Monday the 9th.
    assert_eq!(within("thisWeek", "2026-11-15"), BTreeSet::from([rows[1].clone(), rows[2].clone()]));
    assert_eq!(within("thisWeek", "2026-11-16"), BTreeSet::from([rows[3].clone()]));
    assert_eq!(within("thisMonth", "2026-11-30"), BTreeSet::from_iter(rows[..4].iter().cloned()));
    assert_eq!(within("nextWeek", "2026-11-09"), BTreeSet::from([rows[1].clone(), rows[2].clone(), rows[3].clone()]));
    let error = fixture.notes.query_database(&fixture.db, &QuerySpec { today: Some("tomorrow".into()), ..Default::default() }).unwrap_err().to_string();
    assert!(error.contains("YYYY-MM-DD"), "{error}");
}

#[test]
fn a_view_supplies_what_the_call_leaves_out() {
    let fixture = fixture(40, 8);
    let p = |name: &str| fixture.props[name].clone();
    let view = crate::notes::doc::ViewDef {
        name: "High".into(),
        kind: crate::notes::doc::ViewType::Table,
        order: 5,
        filter: Some(json!({"property": p("Rating"), "op": "gte", "value": 3})),
        sorts: vec![SortDef { property: p("Rating"), direction: "desc".into() }],
        group_by: None,
        date_property: None,
        properties: vec![],
    };
    let view_id = fixture.notes.put_view(&fixture.db, None, view, PERSON).unwrap();
    let saved = query(&fixture, QuerySpec { view_id: Some(view_id.clone()), ..Default::default() });
    assert!(saved.rows.iter().all(|row| row.values[&p("Rating")].as_f64().unwrap() >= 3.0));
    let overridden = query(&fixture, QuerySpec { view_id: Some(view_id), filter: Some(json!({"and": []})), ..Default::default() });
    assert_eq!(overridden.total, 40, "an explicit filter replaces the view's for this call");
    let ratings: Vec<f64> = overridden.rows.iter().filter_map(|row| row.values.get(&p("Rating")).and_then(Value::as_f64)).collect();
    assert!(ratings.windows(2).all(|pair| pair[0] >= pair[1]), "the view's sort still applies");
    let error = fixture.notes.query_database(&fixture.db, &QuerySpec { view_id: Some("nope".into()), ..Default::default() }).unwrap_err().to_string();
    assert!(error.contains("no view nope") && error.contains("High ("), "{error}");
}

#[test]
fn free_text_intersects_with_search() {
    let fixture = fixture(30, 9);
    let result = query(&fixture, QuerySpec { query: Some("alph".into()), ..Default::default() });
    assert!(result.total > 0);
    assert!(result.rows.iter().all(|row| row.note.title.as_deref().unwrap_or_default().to_ascii_lowercase().contains("alph")), "{:?}", result.rows.iter().map(|row| row.note.title.clone()).collect::<Vec<_>>());
}

#[test]
fn the_system_views_return_todays_lists() {
    let (_dir, notes) = service();
    let page = notes.create(CreatePage { title: Some("Plan".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("Inbox".into()), state: Some("inbox".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("Child task".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), state: Some("inbox".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("Top task".into()), kind: Some("task".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("Old".into()), state: Some("archived".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("Old idea".into()), kind: Some("idea".into()), state: Some("archived".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("An idea".into()), kind: Some("idea".into()), ..Default::default() }, PERSON).unwrap();
    let working = notes.create(CreatePage { title: Some("With an agent".into()), kind: Some("task".into()), state: Some("doing".into()), ..Default::default() }, PERSON).unwrap();
    notes.update(UpdateNote { id: working.id, add_session_id: Some("chat".into()), ..Default::default() }, PERSON).unwrap();
    let list = notes.create(CreatePage { title: Some("A database".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
    notes.create(CreatePage { title: Some("A row".into()), parent_id: Some(list.id), ..Default::default() }, PERSON).unwrap();

    let system = notes.system_database().unwrap();
    let all = notes.list(&crate::notes::service::ListFilter { include_archived: true, ..Default::default() }).unwrap();
    let child_task = |note: &Note| note.parent_id.is_some() && note.kind == "task";
    let today = |tab: &str| -> BTreeSet<String> {
        all.iter()
            .filter(|note| match tab {
                "archived" => note.state == "archived",
                "ideas" => note.kind == "idea",
                "agent" => note.state == "doing" && !note.session_ids.is_empty(),
                _ if note.state == "archived" => false,
                "tasks" => note.kind == "task",
                "pages" => note.kind != "task",
                _ => !child_task(note) && (tab == "all" || note.state == "inbox"),
            })
            .map(|note| note.id.clone())
            .collect()
    };
    let database = notes.database(&system).unwrap();
    for (name, tab) in [("All", "all"), ("Inbox", "inbox"), ("Pages", "pages"), ("Tasks", "tasks"), ("Ideas", "ideas"), ("In progress with an agent", "agent"), ("Archived", "archived")] {
        let view = database.views.iter().find(|(_, view)| view.name == name).unwrap().0.clone();
        let result = notes.query_database(&system, &QuerySpec { view_id: Some(view), limit: Some(MAX_LIMIT), ..Default::default() }).unwrap();
        assert_eq!(ids_of(&result.rows), today(tab), "{name}");
        assert!(result.rows.iter().all(|row| row.note.id != system), "the system database is not its own row");
    }
}

/// Three conditions, two sorts and a grouping over 1000 rows. The release
/// bound is the contract's; a debug build only guards against a regression
/// by an order of magnitude, so a slow CI debug run does not flake.
#[test]
fn performance_gate() {
    let (_dir, notes) = service();
    let id = crate::notes::database::tests::database(&notes, "Shelf");
    let added = notes
        .update_schema(
            &id,
            &[
                SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() }),
                SchemaOp::Add(NewProperty { name: "Status".into(), kind: "status".into(), options: options(&[("To do", Some("todo")), ("Reading", Some("inProgress")), ("Done", Some("done"))]), ..Default::default() }),
                SchemaOp::Add(NewProperty { name: "Due".into(), kind: "date".into(), ..Default::default() }),
            ],
            PERSON,
        )
        .unwrap();
    let prop = |name: &str| added.added.iter().find(|(_, added)| added == name).unwrap().0.clone();
    let (rating, status, due) = (prop("Rating"), prop("Status"), prop("Due"));
    let statuses: Vec<String> = notes.database(&id).unwrap().schema[&status].options.iter().map(|option| option.id.clone()).collect();
    for index in 0..1000 {
        let row = notes.create(CreatePage { title: Some(format!("Book {index:04}")), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap().id;
        notes
            .set_values(
                &row,
                &[
                    (rating.clone(), Some(json!((index % 5) as f64 + 1.0))),
                    (status.clone(), Some(json!(statuses[index % 3]))),
                    (due.clone(), Some(json!({"start": format!("2026-{:02}-{:02}", 1 + index % 12, 1 + index % 28)}))),
                ],
                PERSON,
            )
            .unwrap();
    }
    let spec = QuerySpec {
        filter: Some(json!({"and": [
            {"property": rating, "op": "gte", "value": 2},
            {"property": status, "op": "isNot", "value": statuses[2]},
            {"property": due, "op": "onOrAfter", "value": "2026-03-01"},
        ]})),
        sorts: Some(vec![SortDef { property: due.clone(), direction: "desc".into() }, SortDef { property: "title".into(), direction: "asc".into() }]),
        group_by: Some(GroupByDef { property: status.clone(), date_bucket: None }),
        ..Default::default()
    };
    let mut best = std::time::Duration::MAX;
    let mut total = 0;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let result = notes.query_database(&id, &spec).unwrap();
        best = best.min(started.elapsed());
        total = result.total;
    }
    assert!(total > 100, "the filter keeps a real share of the rows: {total}");
    let bound = if cfg!(debug_assertions) { std::time::Duration::from_secs(2) } else { std::time::Duration::from_millis(200) };
    eprintln!("[performance_gate] best of five: {best:?} (bound {bound:?})");
    assert!(best < bound, "1000-row query took {best:?}, over {bound:?}");
}
