//! The size ceiling of a Bridge tool reply.
//!
//! The `bridge` MCP server passes each `tools/call` reply through
//! [`bound_reply`] before the harness reads it. A reply above the ceiling of
//! its tool is cut on a boundary of its structure and says so in `cut`. A tool
//! with pages also gets `next`, the `offset` of the call that returns the rest.
//! The tool applies that offset. The limiter only cuts and says where the next
//! call starts.
//!
//! The size of a reply is the byte length of its text block, so each measure
//! here counts the output of `serde_json::to_string_pretty`, escapes included.
//! Each search is a binary search over a count with a monotone size, so the
//! work grows as n log n, never as n squared.
//!
//! Pure: no I/O, no clock, no database.

use std::collections::HashSet;
use std::io;

use serde::Serialize;
use serde_json::{json, Map, Value};

/// The values are provisional. Each class has its own constant, so one class
/// can change when the size distribution of a large database is known.
pub const LIST_CEILING: usize = 32 * 1024;
pub const DOCUMENT_CEILING: usize = 32 * 1024;
pub const BOUNDED_CEILING: usize = 32 * 1024;
pub const WRITE_CEILING: usize = 32 * 1024;
/// The ceiling of the text of an error reply, for each tool.
pub const ERROR_CEILING: usize = 8 * 1024;

/// The smallest ceiling that holds each fallback. The last fallback needs
/// about 15 bytes, and the fallback of a list about 45 bytes.
const MIN_CEILING: usize = 1024;
const _: () = assert!(LIST_CEILING >= MIN_CEILING);
const _: () = assert!(DOCUMENT_CEILING >= MIN_CEILING);
const _: () = assert!(BOUNDED_CEILING >= MIN_CEILING);
const _: () = assert!(WRITE_CEILING >= MIN_CEILING);
const _: () = assert!(ERROR_CEILING >= MIN_CEILING);

/// The fewest bytes that a shortened text keeps.
const MIN_TEXT: usize = 256;
/// The most shortened parts that `cut` names one by one.
const NAMED_PARTS: usize = 8;
/// The longest pointer that `cut` names. A key can be of any length.
const POINTER_BYTES: usize = 80;
/// Measures of a page after its notice is added. The notice length changes
/// with its numbers, so a page can need a second measure.
const ROUNDS: usize = 8;
const UNCAPPED: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Whole items of the array under `key`. `offset` counts items.
    List { key: &'static str },
    /// A start of the string under `key`. `offset` counts characters.
    Document { key: &'static str },
    /// No pages: a narrower call gets a smaller reply.
    Bounded,
    /// No pages: the reply reports a change that is already made.
    Write,
}

impl Class {
    pub const fn ceiling(self) -> usize {
        match self {
            Class::List { .. } => LIST_CEILING,
            Class::Document { .. } => DOCUMENT_CEILING,
            Class::Bounded => BOUNDED_CEILING,
            Class::Write => WRITE_CEILING,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    pub class: Class,
    pub ceiling: usize,
}

impl Limit {
    pub const fn of(class: Class) -> Self {
        Self { class, ceiling: class.ceiling() }
    }
}

/// Every tool of the `bridge` server and its class.
const TABLE: [(&str, Class); 18] = [
    ("notes_search", Class::List { key: "pages" }),
    ("memory_search", Class::List { key: "records" }),
    ("notes_read", Class::Document { key: "markdown" }),
    ("chats_search", Class::Bounded),
    ("chats_read", Class::Bounded),
    ("settings_read", Class::Bounded),
    (crate::mcp_apps::tool::TOOL_NAME, Class::Bounded),
    ("apps_draft", Class::Write),
    ("apps_open", Class::Write),
    ("notes_create", Class::Write),
    ("notes_edit", Class::Write),
    ("notes_append", Class::Write),
    ("notes_draw", Class::Write),
    ("notes_set", Class::Write),
    ("memory_propose", Class::Write),
    ("outgoing_draft", Class::Write),
    ("settings_change", Class::Write),
    ("notify_user", Class::Write),
];

/// The limit of `tool`, or `None` for a name that is not a Bridge tool.
pub fn limit_of(tool: &str) -> Option<Limit> {
    TABLE.iter().find(|(name, _)| *name == tool).map(|(_, class)| Limit::of(*class))
}

/// The reply that the harness gets for the MCP reply of one `tools/call`. A
/// tool with no entry gets the `Write` limit, so no reply goes out unbounded.
pub fn bound_reply(tool: &str, arguments: &Value, reply: Value) -> Value {
    let limit = limit_of(tool).unwrap_or(Limit::of(Class::Write));
    bound_reply_within(tool, limit, ERROR_CEILING, arguments, reply)
}

/// [`bound_reply`] with the ceilings given, so that a test can use small ones.
/// A ceiling below 1024 bytes is not supported.
pub fn bound_reply_within(tool: &str, limit: Limit, error_ceiling: usize, arguments: &Value, mut reply: Value) -> Value {
    let ceiling = if reply.get("isError").and_then(Value::as_bool) == Some(true) { error_ceiling } else { limit.ceiling };
    let Some(size) = reply.pointer("/content/0/text").and_then(Value::as_str).map(str::len) else {
        return reply;
    };
    if size <= ceiling {
        return reply;
    }
    let text = match reply.get_mut("structuredContent").map(Value::take) {
        Some(structured) => {
            let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let bounded = bound_value(tool, limit.class, ceiling, offset, structured);
            let text = serde_json::to_string_pretty(&bounded).unwrap_or_default();
            reply["structuredContent"] = bounded;
            text
        }
        None => bound_text(reply.pointer("/content/0/text").and_then(Value::as_str).unwrap_or_default(), ceiling),
    };
    if let Some(slot) = reply.pointer_mut("/content/0/text") {
        *slot = Value::String(text);
    }
    reply
}

/// A text with no structure: its longest start that fits, then one last line
/// with the full size.
fn bound_text(text: &str, ceiling: usize) -> String {
    let last = format!("Cut at the ceiling of {ceiling} bytes. The full text has {} bytes.", text.len());
    let kept = cut_text(text, ceiling.saturating_sub(last.len() + 1), 0);
    if kept.is_empty() {
        return char_start(&last, ceiling).to_owned();
    }
    let separator = if kept.ends_with('\n') { "" } else { "\n" };
    format!("{kept}{separator}{last}")
}

/// A structured value above `ceiling`, bounded by its class.
fn bound_value(tool: &str, class: Class, ceiling: usize, offset: u64, value: Value) -> Value {
    if pretty_len(&value) <= ceiling {
        return value;
    }
    // `cut` needs an object to live in.
    let value = match value {
        object @ Value::Object(_) => object,
        other => json!({"result": other}),
    };
    let pager = |key, unit| Pager { tool, key, unit, ceiling, offset, full: 0 };
    match class {
        Class::List { key } if value.get(key).and_then(Value::as_array).is_some_and(|items| !items.is_empty()) => bound_list(pager(key, "item"), value),
        Class::Document { key } if value.get(key).and_then(Value::as_str).is_some_and(|text| !text.is_empty()) => bound_document(pager(key, "character"), value),
        _ => bound_whole(tool, class, ceiling, &value),
    }
}

/// The first items that fit. The other parts get at most half of the ceiling.
fn bound_list(mut pager: Pager<'_>, mut rest: Value) -> Value {
    let key = pager.key;
    let items = match rest.get_mut(key).map(|slot| std::mem::replace(slot, Value::Array(Vec::new()))) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    pager.full = items.len();
    let ceiling = pager.ceiling;
    let (settled, parts) = settle(&rest, key, ceiling);
    let probe = |kept: usize| {
        let page = pager.finish(with_part(settled.clone(), key, Value::Array(items[..kept].to_vec())), kept, &parts);
        (pretty_len(&page) <= ceiling).then_some(page)
    };
    if let Some(page) = search(1, items_within(&items, ceiling) + 1, probe) {
        return page;
    }
    // The first item alone is above the ceiling: it goes out shortened, so
    // the next call still moves on.
    let Some(first) = items.first() else { return pager.none_fits() };
    pager.single(&with_part(without_removed(rest, &parts), key, Value::Array(vec![first.clone()])), &parts).unwrap_or_else(|| pager.too_large(settled, &parts))
}

/// The longest start of the text that fits. The other parts get at most half
/// of the ceiling.
fn bound_document(mut pager: Pager<'_>, mut rest: Value) -> Value {
    let key = pager.key;
    let text = match rest.get_mut(key).map(|slot| std::mem::replace(slot, Value::String(String::new()))) {
        Some(Value::String(text)) => text,
        _ => String::new(),
    };
    pager.full = text.chars().count();
    let (settled, parts) = settle(&rest, key, pager.ceiling);
    if let Some(page) = document_page(&pager, &settled, &text, &parts) {
        return page;
    }
    let first = text.chars().next().map(String::from).unwrap_or_default();
    pager.single(&with_part(without_removed(rest, &parts), key, Value::String(first)), &parts).unwrap_or_else(|| pager.none_fits())
}

/// The page of the longest start that fits. The notice length depends on the
/// count of kept characters, so each round measures the notice for the count
/// of the round before. With the exact count, the measure is exact.
fn document_page(pager: &Pager<'_>, settled: &Value, text: &str, parts: &[Part]) -> Option<Value> {
    let mut guess = pager.full;
    let mut slack = 0;
    let mut best: Option<(usize, Value)> = None;
    for _ in 0..ROUNDS {
        // `settled` holds an empty string under the key, so this is the page
        // with no text and the notice for `guess` characters.
        let overhead = pretty_len(&pager.finish(settled.clone(), guess, parts));
        let Some(room) = pager.ceiling.checked_sub(overhead + slack) else { break };
        let end = prefix_end(text, room);
        let kept = text[..end].chars().count();
        if end == 0 || best.as_ref().is_some_and(|(longest, _)| *longest >= kept) {
            break;
        }
        let page = pager.finish(with_part(settled.clone(), pager.key, Value::String(text[..end].to_owned())), kept, parts);
        let size = pretty_len(&page);
        if size <= pager.ceiling {
            if kept == guess {
                return Some(page);
            }
            // The notice of `guess` can be longer than the notice of `kept`,
            // so a round with the exact count can keep more.
            best = Some((kept, page));
        } else if kept == guess {
            // Not reached while the measure is exact. The slack keeps the
            // rounds from a loop in any case.
            slack += size - pager.ceiling;
        }
        guess = kept;
    }
    best.map(|(_, page)| page)
}

/// A tool with no pages: the whole result, shortened.
fn bound_whole(tool: &str, class: Class, ceiling: usize, value: &Value) -> Value {
    let notice = |parts: &[Part]| format!("Cut at the ceiling of {ceiling} bytes.{}{}{}", after_cut(tool, class), shortened(parts), other_source(tool));
    let shrunk = shrink(value, None, ceiling, &|candidate, parts| pretty_len(&finish(candidate.clone(), notice(parts), None)) <= ceiling);
    if shrunk.fits {
        return finish(shrunk.value, notice(&shrunk.parts), None);
    }
    only_cut(format!("Cut at the ceiling of {ceiling} bytes: the whole result was left out.{}", after_cut(tool, class)), None, ceiling)
}

/// What the agent does after a cut of a tool with no pages.
fn after_cut(tool: &str, class: Class) -> &'static str {
    match class {
        Class::Write => " The change itself is complete: do not send it again.",
        _ if tool == crate::settings_tools::READ_TOOL => " Pass search to narrow it.",
        _ if tool.starts_with(crate::history_tools::PREFIX) => " Use a narrower query.",
        _ => "",
    }
}

/// Where the agent gets a part with no pages that a paged tool shortened.
/// `notes_read` shows the first rows of a database, and `notes_search` pages
/// each row.
fn other_source(tool: &str) -> &'static str {
    if tool == "notes_read" { " For the rows of a database, call notes_search with database." } else { "" }
}

/// The parts with no pages, shortened to half of the ceiling when they are
/// above it. The part with pages under `key` gets the remaining space.
fn settle(rest: &Value, key: &str, ceiling: usize) -> (Value, Vec<Part>) {
    let half = ceiling / 2;
    if pretty_len(rest) <= half {
        return (rest.clone(), Vec::new());
    }
    let fits = |candidate: &Value, _: &[Part]| pretty_len(candidate) <= half;
    let shrunk = shrink(rest, None, half, &fits);
    let Value::Object(map) = rest else { return (shrunk.value, shrunk.parts) };
    if shrunk.fits {
        return (shrunk.value, shrunk.parts);
    }
    // Some parts are too large even with each text and list empty. Whole
    // parts go, so that the part with pages keeps its room. `shrunk` holds
    // each part at the size that it cannot go below, and the largest of those
    // goes first. A part that only had a long text stays.
    let floor = |name: &String| shrunk.value.get(name.as_str()).map_or(0, pretty_len);
    let mut order: Vec<(usize, &String)> = map.keys().filter(|name| *name != key).map(|name| (floor(name), name)).collect();
    order.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    let probe = |removed: usize| {
        let gone: HashSet<&String> = order[..removed].iter().map(|(_, name)| *name).collect();
        let is_gone = |name: &String| gone.contains(name);
        let candidate: Map<String, Value> = map.iter().filter(|(name, _)| !is_gone(name)).map(|(name, part)| (name.clone(), part.clone())).collect();
        let shrunk = shrink(&Value::Object(candidate), None, half, &fits);
        shrunk.fits.then(|| {
            let mut parts = shrunk.parts;
            parts.extend(map.keys().filter(|name| is_gone(name)).map(|name| Part::Removed { key: name.clone() }));
            (shrunk.value, parts)
        })
    };
    // `search` gives the largest count that fits, and the fewest removed parts
    // are wanted, so it counts the parts that stay.
    search(0, order.len(), |kept| probe(order.len() - kept)).unwrap_or((shrunk.value, shrunk.parts))
}

/// `value` with each part that [`settle`] removed left out.
fn without_removed(mut value: Value, parts: &[Part]) -> Value {
    if let Some(map) = value.as_object_mut() {
        let gone: HashSet<&str> = parts.iter().filter_map(|part| if let Part::Removed { key } = part { Some(key.as_str()) } else { None }).collect();
        map.retain(|name, _| !gone.contains(name.as_str()));
    }
    value
}

/// The count of leading items whose compact size fits in `ceiling`. The
/// pretty form is never smaller, so more items never fit.
fn items_within(items: &[Value], ceiling: usize) -> usize {
    let mut total = 0usize;
    items
        .iter()
        .take_while(|item| {
            total = total.saturating_add(compact_len(item));
            total <= ceiling
        })
        .count()
}

struct Pager<'a> {
    tool: &'a str,
    key: &'static str,
    unit: &'static str,
    ceiling: usize,
    offset: u64,
    /// Units of the part with pages in the reply of this call.
    full: usize,
}

impl Pager<'_> {
    fn next(&self, kept: usize) -> u64 {
        self.offset.saturating_add(u64::try_from(kept).unwrap_or(u64::MAX))
    }

    fn notice(&self, kept: usize, parts: &[Part]) -> String {
        format!(
            "Cut at the ceiling of {} bytes: this reply holds {} and leaves out {}. Call {} again with offset {} and the same other arguments for the rest.{}",
            self.ceiling,
            count(kept, self.unit),
            self.full.saturating_sub(kept),
            self.tool,
            self.next(kept),
            shortened(parts)
        )
    }

    /// A page of `kept` units. When the part with pages is whole, only other
    /// parts are shortened, and a next call has nothing more to return.
    fn finish(&self, value: Value, kept: usize, parts: &[Part]) -> Value {
        if kept == self.full {
            let notice = format!("Cut at the ceiling of {} bytes: this reply holds all {}, and no call with offset returns more.{}{}", self.ceiling, count(kept, self.unit), shortened(parts), other_source(self.tool));
            return finish(value, notice, None);
        }
        self.paged(value, kept, parts)
    }

    fn paged(&self, value: Value, kept: usize, parts: &[Part]) -> Value {
        finish(value, self.notice(kept, parts), Some(self.next(kept)))
    }

    /// One unit of the part with pages, with the whole reply shortened
    /// around it. `settled` names the parts that [`settle`] removed, and
    /// `candidate` is already without them.
    fn single(&self, candidate: &Value, settled: &[Part]) -> Option<Value> {
        let removed: Vec<Part> = settled.iter().filter(|part| matches!(part, Part::Removed { .. })).cloned().collect();
        let all = |parts: &[Part]| [parts, &removed].concat();
        // The reply gets a next only when it leaves out units of this call.
        // A lone unit has nothing after it, shortened or not.
        let page = |value: Value, parts: &[Part]| self.finish(value, 1, &all(parts));
        let shrunk = shrink(candidate, Some(self.key), self.ceiling, &|value, parts| pretty_len(&page(value.clone(), parts)) <= self.ceiling);
        shrunk.fits.then(|| page(shrunk.value, &shrunk.parts))
    }

    /// The first item is above the ceiling even with each of its texts and
    /// lists empty. The reply names it and the next call starts after it.
    fn too_large(&self, settled: Value, parts: &[Part]) -> Value {
        let notice = |parts: &[Part]| {
            format!(
                "Cut at the ceiling of {} bytes: the {} at offset {} is too large for one reply and is not in the reply. This reply holds 0 {}s and leaves out {}. Call {} again with offset {} and the same other arguments for the rest.{}",
                self.ceiling,
                self.unit,
                self.offset,
                self.unit,
                self.full,
                self.tool,
                self.next(1),
                shortened(parts)
            )
        };
        let page = finish(with_part(settled, self.key, Value::Array(Vec::new())), notice(parts), Some(self.next(1)));
        if pretty_len(&page) <= self.ceiling {
            return page;
        }
        only_cut(notice(&[]), Some(self.next(1)), self.ceiling)
    }

    fn none_fits(&self) -> Value {
        let text = format!(
            "Cut at the ceiling of {} bytes: not even one {} fits, so this reply leaves out all {}. Call {} again with offset {} to skip it.",
            self.ceiling,
            self.unit,
            count(self.full, self.unit),
            self.tool,
            self.next(1)
        );
        only_cut(text, Some(self.next(1)), self.ceiling)
    }
}

fn count(amount: usize, unit: &str) -> String {
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}

fn shortened(parts: &[Part]) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let mut named: Vec<String> = parts.iter().take(NAMED_PARTS).map(Part::describe).collect();
    if parts.len() > NAMED_PARTS {
        named.push(format!("and {} more", parts.len() - NAMED_PARTS));
    }
    format!(" Shortened: {}.", named.join(", "))
}

fn with_part(mut value: Value, key: &str, part: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_owned(), part);
    }
    value
}

/// `value` with `cut` and `next` as its last keys.
fn finish(value: Value, cut: String, next: Option<u64>) -> Value {
    let mut map = match value {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.shift_remove("cut");
    map.shift_remove("next");
    map.insert("cut".to_owned(), Value::String(cut));
    if let Some(offset) = next {
        map.insert("next".to_owned(), json!({"offset": offset}));
    }
    Value::Object(map)
}

/// The last fallback: an object that holds only `cut` (and `next` for a
/// tool with pages).
fn only_cut(text: String, next: Option<u64>, ceiling: usize) -> Value {
    let room = ceiling.saturating_sub(pretty_len(&finish(Value::Object(Map::new()), String::new(), next)));
    let end = fitting_end(&text, room);
    finish(Value::Object(Map::new()), text[..end].to_owned(), next)
}

/// A part with no pages that the limiter shortened, by JSON pointer. `cut`
/// names only the first [`NAMED_PARTS`], so the parts after them have an empty
/// pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Items { pointer: String, kept: usize, full: usize },
    Text { pointer: String, kept: usize, full: usize },
    /// A top-level part that is left out whole.
    Removed { key: String },
}

impl Part {
    fn describe(&self) -> String {
        let (pointer, kept, full, unit) = match self {
            Part::Items { pointer, kept, full } => (pointer, kept, full, "items"),
            Part::Text { pointer, kept, full } => (pointer, kept, full, "bytes"),
            Part::Removed { key } => return format!("{} is left out", char_start(&escape_key(key), POINTER_BYTES)),
        };
        format!("{} keeps {kept} of {full} {unit}", char_start(pointer, POINTER_BYTES))
    }
}

/// One step of a JSON pointer for an object key.
fn escape_key(key: &str) -> String {
    format!("/{}", key.replace('~', "~0").replace('/', "~1"))
}

/// How much of each part with no pages a candidate keeps.
#[derive(Debug, Clone, Copy)]
struct Caps {
    /// The most items of each list.
    items: usize,
    /// The most bytes of each text.
    bytes: usize,
}

struct Shrunk {
    value: Value,
    parts: Vec<Part>,
    fits: bool,
}

/// Shorten the parts with no pages of `value` until `fits` holds: whole items
/// from the end of the longest lists (one stays in each), then the longest
/// texts (at least [`MIN_TEXT`] bytes stay), then the remaining items, then
/// each text below [`MIN_TEXT`] bytes, down to empty. The top-level key
/// `protected` keeps all of its own items or text. When nothing fits, the
/// result has `fits` false and the caller falls back.
fn shrink(value: &Value, protected: Option<&str>, ceiling: usize, fits: &dyn Fn(&Value, &[Part]) -> bool) -> Shrunk {
    let (longest_list, longest_text) = extent(value, protected);
    // A list item takes at least four bytes in the pretty form, and a text
    // longer than the ceiling never fits. So larger caps never fit, and each
    // probe stays near the size of the ceiling.
    let list_high = longest_list.min(ceiling / 4 + 1);
    let text_high = longest_text.min(ceiling.saturating_add(8));
    let probe = |items: usize, bytes: usize| {
        let (candidate, parts) = apply(value, Caps { items, bytes }, protected);
        fits(&candidate, &parts).then_some(Shrunk { value: candidate, parts, fits: true })
    };
    probe(UNCAPPED, UNCAPPED)
        .or_else(|| search(1, list_high, |items| probe(items, UNCAPPED)))
        .or_else(|| search(MIN_TEXT, text_high, |bytes| probe(1, bytes)))
        .or_else(|| probe(0, UNCAPPED))
        .or_else(|| search(MIN_TEXT, text_high, |bytes| probe(0, bytes)))
        .or_else(|| search(0, MIN_TEXT.min(text_high), |bytes| probe(0, bytes)))
        .unwrap_or_else(|| {
            let (value, parts) = apply(value, Caps { items: 0, bytes: 0 }, protected);
            Shrunk { value, parts, fits: false }
        })
}

/// The largest `n` in `low..high` for which `probe` gives a value, if
/// `probe(low)` gives one. The caller knows that `probe(high)` gives none.
fn search<T>(low: usize, high: usize, probe: impl Fn(usize) -> Option<T>) -> Option<T> {
    if low >= high {
        return None;
    }
    let mut best = probe(low)?;
    let (mut fits, mut fails) = (low, high);
    while fails - fits > 1 {
        let middle = fits + (fails - fits) / 2;
        match probe(middle) {
            Some(found) => {
                best = found;
                fits = middle;
            }
            None => fails = middle,
        }
    }
    Some(best)
}

/// The longest list and the longest text in bytes, outside `protected`.
fn extent(value: &Value, protected: Option<&str>) -> (usize, usize) {
    fn walk(value: &Value, exempt: bool, extent: &mut (usize, usize)) {
        match value {
            Value::Array(items) => {
                if !exempt {
                    extent.0 = extent.0.max(items.len());
                }
                items.iter().for_each(|item| walk(item, false, extent));
            }
            Value::Object(map) => map.values().for_each(|item| walk(item, false, extent)),
            Value::String(text) if !exempt => extent.1 = extent.1.max(text.len()),
            _ => {}
        }
    }
    let mut extent = (0, 0);
    match value {
        Value::Object(map) => map.iter().for_each(|(key, item)| walk(item, Some(key.as_str()) == protected, &mut extent)),
        other => walk(other, false, &mut extent),
    }
    extent
}

fn apply(value: &Value, caps: Caps, protected: Option<&str>) -> (Value, Vec<Part>) {
    let mut cutter = Cutter { caps, protected, path: Vec::new(), parts: Vec::new() };
    let value = cutter.cut(value);
    (value, cutter.parts)
}

enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

/// Copies a value with its caps applied, and records each part it shortens
/// in document order.
struct Cutter<'a> {
    caps: Caps,
    protected: Option<&'a str>,
    path: Vec<Step<'a>>,
    parts: Vec<Part>,
}

impl<'a> Cutter<'a> {
    fn exempt(&self) -> bool {
        matches!((self.path.as_slice(), self.protected), ([Step::Key(key)], Some(protected)) if *key == protected)
    }

    /// The pointer of the next part, or an empty one when `cut` only counts it.
    fn named(&self) -> String {
        if self.parts.len() >= NAMED_PARTS {
            return String::new();
        }
        self.path
            .iter()
            .map(|step| match step {
                Step::Key(key) => escape_key(key),
                Step::Index(index) => format!("/{index}"),
            })
            .collect()
    }

    fn cut(&mut self, value: &'a Value) -> Value {
        match value {
            Value::Array(items) => {
                let kept = if self.exempt() { items.len() } else { items.len().min(self.caps.items) };
                if kept < items.len() {
                    self.parts.push(Part::Items { pointer: self.named(), kept, full: items.len() });
                }
                let mut out = Vec::with_capacity(kept);
                for (index, item) in items.iter().take(kept).enumerate() {
                    self.path.push(Step::Index(index));
                    out.push(self.cut(item));
                    self.path.pop();
                }
                Value::Array(out)
            }
            Value::Object(map) => {
                let mut out = Map::new();
                for (key, item) in map {
                    self.path.push(Step::Key(key));
                    out.insert(key.clone(), self.cut(item));
                    self.path.pop();
                }
                Value::Object(out)
            }
            Value::String(text) if text.len() > self.caps.bytes && !self.exempt() => {
                let kept = cut_text(text, self.caps.bytes, MIN_TEXT);
                if kept.len() < text.len() {
                    self.parts.push(Part::Text { pointer: self.named(), kept: kept.len(), full: text.len() });
                }
                Value::String(kept.to_owned())
            }
            other => other.clone(),
        }
    }
}

/// The longest start of `text` of at most `max_bytes` bytes. It ends just
/// after a line break when one keeps at least `min_keep` bytes, else at a
/// character boundary. When `max_bytes` is at least `min_keep`, the start
/// keeps at least `min_keep` bytes, so it can end up to 3 bytes after
/// `max_bytes` in a text of wide characters.
fn cut_text(text: &str, max_bytes: usize, min_keep: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let start = char_start(text, max_bytes);
    match start.rfind('\n') {
        Some(at) if at + 1 >= min_keep => &text[..=at],
        _ if start.len() < min_keep && max_bytes >= min_keep => {
            let end = (min_keep..=text.len()).find(|end| text.is_char_boundary(*end)).unwrap_or(text.len());
            &text[..end]
        }
        _ => start,
    }
}

/// The longest start of `text` of at most `max_bytes` bytes that ends at a
/// character boundary.
fn char_start(text: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The byte end of the longest start of `text` whose JSON escape fits in
/// `room` bytes.
fn fitting_end(text: &str, room: usize) -> usize {
    let mut used = 0usize;
    for (at, character) in text.char_indices() {
        used += escaped_len(character);
        if used > room {
            return at;
        }
    }
    text.len()
}

/// [`fitting_end`], moved back to just after the last line break of that
/// start when it does not hold the whole text.
fn prefix_end(text: &str, room: usize) -> usize {
    let end = fitting_end(text, room);
    if end == text.len() {
        return end;
    }
    text[..end].rfind('\n').map_or(end, |at| at + 1)
}

/// Bytes of one character inside a JSON string, as serde_json writes it.
fn escaped_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        control if u32::from(control) < 0x20 => 6,
        other => other.len_utf8(),
    }
}

struct Counter(usize);

impl io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The byte length of `serde_json::to_string_pretty(value)`, with no buffer.
fn pretty_len(value: &Value) -> usize {
    let mut counter = Counter(0);
    let mut serializer = serde_json::Serializer::with_formatter(&mut counter, serde_json::ser::PrettyFormatter::new());
    // A counter never fails, and a `Value` always serializes.
    let written = value.serialize(&mut serializer).is_ok();
    debug_assert!(written);
    counter.0
}

fn compact_len(value: &Value) -> usize {
    let mut counter = Counter(0);
    let written = serde_json::to_writer(&mut counter, value).is_ok();
    debug_assert!(written);
    counter.0
}

#[cfg(test)]
#[path = "tool_limit/tests.rs"]
mod tests;
