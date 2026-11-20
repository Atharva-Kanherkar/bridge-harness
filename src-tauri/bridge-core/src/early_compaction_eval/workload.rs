//! The workload of the live eval and the rubric of its last turns.
//!
//! `testing/evals/early-compaction/workload.json` holds a fixture repository
//! and 14 scripted turns. Turns 1 to 3 state decisions in the conversation
//! only. Turns 4 to 10 read the generated data files and edit `NOTES.md`, so
//! the window grows past `THRESHOLD_TOKENS`. Turns 11 to 14 make a module that
//! needs the decisions of turns 1 to 3. The data files are not in the JSON:
//! [`generated_text`] makes them from a seed, so the fixture stays small.

use std::path::{Component, Path};

use regex::Regex;
use serde::{Deserialize, Serialize};

const WORKLOAD: &str = include_str!("../../../../testing/evals/early-compaction/workload.json");
/// The workload has this number of turns.
pub(super) const TURNS: usize = 14;
/// The turns that state decisions, by number.
const DECIDE: std::ops::RangeInclusive<usize> = 1..=3;
/// The turns that grow the window, by number.
const GROW: std::ops::RangeInclusive<usize> = 4..=10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Workload {
    pub fixture: Fixture,
    pub decisions: Vec<Decision>,
    pub turns: Vec<Turn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Fixture {
    pub files: Vec<FixtureFile>,
    pub words: Vec<String>,
    pub statuses: Vec<String>,
    pub generated: Vec<Generated>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FixtureFile {
    pub path: String,
    pub text: String,
}

/// A data file that [`generated_text`] makes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Generated {
    pub path: String,
    pub seed: u64,
    pub lines: usize,
    pub words_per_line: usize,
}

/// A decision of turns 1 to 3, and the checks that show that a file keeps it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Decision {
    pub turn: usize,
    pub name: String,
    pub checks: Vec<Check>,
}

/// The file text must match `pattern` when `present` is true, and must not
/// match it when `present` is false. The check reads the text with no
/// comments, so a rule only in a comment gives no point.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Check {
    pub pattern: String,
    pub present: bool,
    /// The check reads the comments too. Only the header rule needs them,
    /// because its line is a comment.
    #[serde(default)]
    pub with_comments: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Phase {
    Decide,
    Grow,
    Recall,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Turn {
    pub phase: Phase,
    pub text: String,
    /// The command of R1, run in the chat directory. Only a recall turn.
    #[serde(default)]
    pub check: Option<Vec<String>>,
    /// The file that R2 reads. Only a recall turn.
    #[serde(default)]
    pub file: Option<String>,
    /// A file that keeps each decision and passes the check, for the unit
    /// tests. Only a recall turn.
    #[serde(default)]
    pub reference: Option<String>,
}

/// The compiled checks of each decision: the pattern, `present` and
/// `with_comments`.
pub(super) struct Rubric {
    decisions: Vec<(String, Vec<(Regex, bool, bool)>)>,
}

/// The points of one recall turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnScore {
    /// The turn number, from 1.
    pub turn: usize,
    /// R1: the check command passed.
    pub check_passed: bool,
    /// The name of each decision and whether the file keeps it.
    pub kept: Vec<(String, bool)>,
    /// The last lines that the check command wrote.
    pub check_output: String,
}

impl TurnScore {
    /// R1 gives 1 point when the check command passes.
    pub fn r1(&self) -> u32 {
        u32::from(self.check_passed)
    }

    /// R2 gives 1 point when the file keeps each decision.
    pub fn r2(&self) -> u32 {
        u32::from(!self.kept.is_empty() && self.kept.iter().all(|(_, kept)| *kept))
    }

    pub fn points(&self) -> u32 {
        self.r1() + self.r2()
    }
}

/// The parsed workload. It fails when the shape of the turns is not the
/// shape that the contract names.
pub(super) fn load() -> Result<Workload, String> {
    let workload: Workload =
        serde_json::from_str(WORKLOAD).map_err(|error| format!("workload.json: {error}"))?;
    workload.validate()?;
    Ok(workload)
}

impl Workload {
    fn validate(&self) -> Result<(), String> {
        if self.turns.len() != TURNS {
            return Err(format!("the workload has {} turns, not {TURNS}", self.turns.len()));
        }
        for (index, turn) in self.turns.iter().enumerate() {
            let number = index + 1;
            let phase = if DECIDE.contains(&number) {
                Phase::Decide
            } else if GROW.contains(&number) {
                Phase::Grow
            } else {
                Phase::Recall
            };
            if turn.phase != phase {
                return Err(format!("turn {number} must be {phase:?}"));
            }
            let scored = turn.check.as_ref().is_some_and(|check| !check.is_empty()) && turn.file.is_some();
            if scored != (phase == Phase::Recall) {
                return Err(format!("turn {number}: only a recall turn has a check and a file"));
            }
        }
        if self.fixture.words.is_empty() || self.fixture.statuses.is_empty() {
            return Err("the fixture needs words and statuses".into());
        }
        if self.decisions.iter().any(|decision| !DECIDE.contains(&decision.turn)) {
            return Err("each decision belongs to turns 1 to 3".into());
        }
        Ok(())
    }

    /// The decision checks, compiled.
    pub fn rubric(&self) -> Result<Rubric, String> {
        let decisions = self
            .decisions
            .iter()
            .map(|decision| {
                let checks = decision
                    .checks
                    .iter()
                    .map(|check| {
                        Regex::new(&check.pattern)
                            .map(|regex| (regex, check.present, check.with_comments))
                            .map_err(|error| format!("{}: {error}", decision.name))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((decision.name.clone(), checks))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Rubric { decisions })
    }

    /// The highest score of one arm: 2 points for each recall turn.
    pub fn max_points(&self) -> u32 {
        let recall = self.turns.iter().filter(|turn| turn.phase == Phase::Recall).count();
        u32::try_from(recall * 2).unwrap_or(u32::MAX)
    }

    /// Write the fixture files and the generated data files below `root`.
    pub fn write_fixture(&self, root: &Path) -> Result<(), String> {
        let literal = self.fixture.files.iter().map(|file| (file.path.as_str(), file.text.clone()));
        let generated = self.fixture.generated.iter().map(|spec| {
            (spec.path.as_str(), generated_text(spec, &self.fixture.words, &self.fixture.statuses))
        });
        for (path, text) in literal.chain(generated) {
            write_below(root, path, &text)?;
        }
        Ok(())
    }
}

fn write_below(root: &Path, relative: &str, text: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if !path.components().all(|component| matches!(component, Component::Normal(_))) {
        return Err(format!("the fixture path {relative} must stay below the chat directory"));
    }
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{relative}: {error}"))?;
    }
    std::fs::write(&target, text).map_err(|error| format!("{relative}: {error}"))
}

/// One data file: each line is a number, `words_per_line` words and a
/// status. The words come from a linear congruential generator, so a seed
/// always gives the same text.
pub(super) fn generated_text(spec: &Generated, words: &[String], statuses: &[String]) -> String {
    let mut state = spec.seed;
    let mut next = |count: usize| -> usize {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(state >> 33).unwrap_or(0) % count.max(1)
    };
    let mut text = String::new();
    for line in 1..=spec.lines {
        text.push_str(&format!("{line:04}"));
        for _ in 0..spec.words_per_line {
            text.push(' ');
            text.push_str(words.get(next(words.len())).map_or("", String::as_str));
        }
        text.push(' ');
        text.push_str(statuses.get(next(statuses.len())).map_or("", String::as_str));
        text.push('\n');
    }
    text
}

/// `text` with no line comment and no block comment. A quoted string or a
/// template keeps its text, so `'http://'` stays. A block comment becomes one
/// space, so the code on its two sides stays apart.
fn strip_comments(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(open) = quote {
            code.push(c);
            if c == '\\' {
                code.extend(chars.next());
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match (c, chars.peek().copied()) {
            ('/', Some('/')) => {
                while chars.next_if(|next| *next != '\n').is_some() {}
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = '\0';
                for next in chars.by_ref() {
                    if last == '*' && next == '/' {
                        break;
                    }
                    last = next;
                }
                code.push(' ');
            }
            ('"' | '\'' | '`', _) => {
                quote = Some(c);
                code.push(c);
            }
            _ => code.push(c),
        }
    }
    code
}

impl Rubric {
    /// Whether `text` keeps each decision. A missing file keeps none.
    pub fn keeps(&self, text: Option<&str>) -> Vec<(String, bool)> {
        let code = text.map(strip_comments);
        self.decisions
            .iter()
            .map(|(name, checks)| {
                let kept = text.zip(code.as_deref()).is_some_and(|(text, code)| {
                    checks.iter().all(|(regex, present, with_comments)| {
                        regex.is_match(if *with_comments { text } else { code }) == *present
                    })
                });
                (name.clone(), kept)
            })
            .collect()
    }
}

/// The rubric points of one recall turn.
pub(super) fn score_turn(turn: usize, check_passed: bool, kept: Vec<(String, bool)>, check_output: String) -> TurnScore {
    TurnScore { turn, check_passed, kept, check_output }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recall(workload: &Workload) -> impl Iterator<Item = (usize, &Turn)> {
        workload
            .turns
            .iter()
            .enumerate()
            .filter(|(_, turn)| turn.phase == Phase::Recall)
            .map(|(index, turn)| (index + 1, turn))
    }

    #[test]
    fn the_workload_has_three_decisions_seven_growth_turns_and_four_recall_turns() -> Result<(), String> {
        let workload = load()?;
        let phases: Vec<Phase> = workload.turns.iter().map(|turn| turn.phase).collect();
        let mut expected = vec![Phase::Decide; 3];
        expected.extend([Phase::Grow; 7]);
        expected.extend([Phase::Recall; 4]);
        if phases != expected {
            return Err(format!("phases {phases:?}"));
        }
        if workload.max_points() != 8 {
            return Err(format!("max points {}", workload.max_points()));
        }
        let decided: Vec<usize> = workload.decisions.iter().map(|decision| decision.turn).collect();
        if decided != [1, 2, 3] {
            return Err(format!("decision turns {decided:?}"));
        }
        // Each growth turn reads its own data file.
        for (index, spec) in workload.fixture.generated.iter().enumerate() {
            let turn = workload.turns.get(index + 3).ok_or("no growth turn")?;
            if turn.phase != Phase::Grow || !turn.text.contains(&spec.path) {
                return Err(format!("turn {} does not read {}", index + 4, spec.path));
            }
        }
        Ok(())
    }

    #[test]
    fn the_generator_is_deterministic_and_uses_only_the_word_list() -> Result<(), String> {
        let workload = load()?;
        let fixture = &workload.fixture;
        let first = fixture.generated.first().ok_or("no generated file")?;
        let text = generated_text(first, &fixture.words, &fixture.statuses);
        if text != generated_text(first, &fixture.words, &fixture.statuses) {
            return Err("the same seed gave two texts".into());
        }
        let other = Generated {
            path: first.path.clone(),
            seed: first.seed + 100,
            lines: first.lines,
            words_per_line: first.words_per_line,
        };
        if text == generated_text(&other, &fixture.words, &fixture.statuses) {
            return Err("two seeds gave the same text".into());
        }
        if text.lines().count() != first.lines {
            return Err(format!("{} lines, not {}", text.lines().count(), first.lines));
        }
        for line in text.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            let (status, words) = fields.split_last().ok_or("empty line")?;
            let words = words.get(1..).ok_or("no words")?;
            if words.len() != first.words_per_line
                || !words.iter().all(|word| fixture.words.iter().any(|known| known == word))
                || !fixture.statuses.iter().any(|known| known == status)
            {
                return Err(format!("line {line:?} is not from the lists"));
            }
        }
        Ok(())
    }

    #[test]
    fn the_growth_files_carry_the_window_past_the_threshold() -> Result<(), String> {
        // A line costs about one token for each word, plus about three for
        // the number, the status and the line end. The smallest base of a
        // live chat was about 29,000 tokens (Codex). The window must pass the
        // threshold by a margin, and the data alone must stay below it, so
        // that one run stays cheap.
        const BASE: usize = 29_000;
        const MARGIN: usize = 10_000;
        let workload = load()?;
        let tokens: usize = workload
            .fixture
            .generated
            .iter()
            .map(|spec| spec.lines * (spec.words_per_line + 3))
            .sum();
        let threshold = usize::try_from(crate::early_compaction::THRESHOLD_TOKENS).map_err(|error| error.to_string())?;
        if BASE + tokens < threshold + MARGIN || tokens > threshold {
            return Err(format!("the data files hold about {tokens} tokens"));
        }
        Ok(())
    }

    #[test]
    fn the_fixture_writes_below_the_root_and_refuses_an_escape() -> Result<(), String> {
        let workload = load()?;
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        workload.write_fixture(root.path())?;
        for file in &workload.fixture.files {
            let text = std::fs::read_to_string(root.path().join(&file.path)).map_err(|error| error.to_string())?;
            if text != file.text {
                return Err(format!("{} changed", file.path));
            }
        }
        for spec in &workload.fixture.generated {
            if !root.path().join(&spec.path).is_file() {
                return Err(format!("{} is missing", spec.path));
            }
        }
        if write_below(root.path(), "../outside.txt", "x").is_ok() {
            return Err("a path above the root was written".into());
        }
        Ok(())
    }

    #[test]
    fn each_reference_keeps_each_decision() -> Result<(), String> {
        let workload = load()?;
        let rubric = workload.rubric()?;
        for (number, turn) in recall(&workload) {
            let reference = turn.reference.as_deref().ok_or(format!("turn {number} has no reference"))?;
            let score = score_turn(number, true, rubric.keeps(Some(reference)), String::new());
            if score.points() != 2 {
                return Err(format!("turn {number}: {score:?}"));
            }
        }
        Ok(())
    }

    #[test]
    fn the_rubric_takes_a_point_for_each_lost_decision_and_for_a_failed_check() -> Result<(), String> {
        let workload = load()?;
        let rubric = workload.rubric()?;
        let (number, turn) = recall(&workload).next().ok_or("no recall turn")?;
        let reference = turn.reference.as_deref().ok_or("no reference")?;
        let broken = [
            reference.replacen("// owner: ledger-team\n", "", 1),
            reference.replace("roundHalfEven(", "Math.round("),
            reference.replace("new LedgerError(", "new Error("),
        ];
        for (index, text) in broken.iter().enumerate() {
            let kept = rubric.keeps(Some(text));
            let lost: Vec<usize> = kept.iter().enumerate().filter(|(_, (_, kept))| !kept).map(|(at, _)| at).collect();
            let score = score_turn(number, true, kept, String::new());
            if lost != [index] || score.r2() != 0 || score.r1() != 1 {
                return Err(format!("break {index}: lost {lost:?}, {score:?}"));
            }
        }
        let missing = score_turn(number, false, rubric.keeps(None), "no file".into());
        if missing.points() != 0 || missing.kept.iter().any(|(_, kept)| *kept) {
            return Err(format!("a missing file scored {missing:?}"));
        }
        Ok(())
    }

    /// The names of the decisions that `text` loses.
    fn lost(rubric: &Rubric, text: &str) -> Vec<String> {
        rubric.keeps(Some(text)).into_iter().filter(|(_, kept)| !kept).map(|(name, _)| name).collect()
    }

    #[test]
    fn a_rule_only_in_a_comment_gives_no_point() -> Result<(), String> {
        let workload = load()?;
        let rubric = workload.rubric()?;
        let (_, turn) = recall(&workload).next().ok_or("no recall turn")?;
        let reference = turn.reference.as_deref().ok_or("no reference")?;
        // The rounding rule only in a block comment, the error rule only in a
        // line comment.
        let commented = [
            (reference.replace("return roundHalfEven(", "/* roundHalfEven( */ return Math.floor(0.5 + "), "rounding"),
            (reference.replace("throw new LedgerError(", "// new LedgerError(\n    throw fail("), "errors"),
        ];
        for (text, name) in &commented {
            if lost(&rubric, text) != [*name] {
                return Err(format!("{name} only in a comment: lost {:?}\n{text}", lost(&rubric, text)));
            }
        }
        // A forbidden call only in a comment costs no point, and a string
        // that holds two slashes is not a comment.
        let kept = [
            reference.replace("  return roundHalfEven(", "  // Not Math.round( or .toFixed(.\n  /* new Error( */\n  return roundHalfEven("),
            reference.replace("return roundHalfEven(", "const note = 'see http://ledger'; return roundHalfEven("),
        ];
        for text in &kept {
            if !lost(&rubric, text).is_empty() {
                return Err(format!("lost {:?}\n{text}", lost(&rubric, text)));
            }
        }
        Ok(())
    }

    #[test]
    fn a_forbidden_rounding_by_its_quoted_name_gives_no_point() -> Result<(), String> {
        let workload = load()?;
        let rubric = workload.rubric()?;
        let (number, turn) = recall(&workload).next().ok_or("no recall turn")?;
        let reference = turn.reference.as_deref().ok_or("no reference")?;
        for form in [
            "Math[\"round\"](",
            "Math['round'](",
            "Math [ 'round' ] (",
            "Number((1.5)[\"toFixed\"](0)) * 0 + (",
            "Number((1.5)['toFixed'](0)) * 0 + (",
        ] {
            let text = reference.replace("return roundHalfEven(", &format!("return roundHalfEven(0) + {form}"));
            let score = score_turn(number, true, rubric.keeps(Some(&text)), String::new());
            if lost(&rubric, &text) != ["rounding"] || score.r2() != 0 {
                return Err(format!("{form}: lost {:?}, {score:?}", lost(&rubric, &text)));
            }
        }
        Ok(())
    }
}
