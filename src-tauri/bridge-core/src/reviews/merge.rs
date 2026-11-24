//! Merging two reviewers' findings (a second opinion).
//!
//! Two passes of one job review the same prepared PR on different harnesses.
//! A finding both flagged becomes one finding with `agreed_by = 2`, so it ranks
//! first in the draft. Pure: the runner reads the stored findings, asks for the
//! merges, and the store writes them under the job's claim.

use std::collections::HashSet;

use super::contract::severity_rank;
use super::steps::DraftFinding;

/// Ranges further apart than this are about different code.
pub const MAX_LINE_GAP: u32 = 3;
/// Title-word overlap (Jaccard) at or above which two near findings match.
pub const MIN_TITLE_OVERLAP: f64 = 0.5;
/// Confidences further apart than this pick the more confident body.
const CONFIDENCE_GAP: f64 = 0.2;

/// Words too common in finding titles to say what a finding is about.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "for", "from", "if", "in", "is", "it", "its", "of", "on", "or",
    "the", "this", "to", "when", "with",
];

/// One stored finding of one pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub pass_id: String,
    pub finding: DraftFinding,
}

/// Two findings that are one: `keep` stays (with the body chosen for it),
/// `drop` goes, and the kept one takes `severity` and `confidence`.
#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    pub keep: String,
    pub drop: String,
    pub severity: bridge_protocol::messages::ReviewSeverity,
    pub confidence: f64,
    /// Both passes, the kept finding's first.
    pub pass_ids: [String; 2],
}

/// The merges among `entries`, one to one, best pair first: equal
/// fingerprints, then higher title overlap, then nearer lines.
pub fn merges(entries: &[Entry]) -> Vec<Merge> {
    let mut candidates = Vec::new();
    for (left_index, left) in entries.iter().enumerate() {
        for (right_index, right) in entries.iter().enumerate().skip(left_index + 1) {
            if left.pass_id == right.pass_id || left.finding.path != right.finding.path {
                continue;
            }
            let gap = line_gap(&left.finding, &right.finding);
            if gap > MAX_LINE_GAP {
                continue;
            }
            let same_fingerprint = left.finding.fingerprint == right.finding.fingerprint;
            let overlap = title_overlap(&left.finding.title, &right.finding.title);
            if same_fingerprint || overlap >= MIN_TITLE_OVERLAP {
                candidates.push((same_fingerprint, overlap, gap, left_index, right_index));
            }
        }
    }
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.total_cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then((a.3, a.4).cmp(&(b.3, b.4)))
    });
    let mut used = HashSet::new();
    let mut merges = Vec::new();
    for (_, _, _, left_index, right_index) in candidates {
        if used.contains(&left_index) || used.contains(&right_index) {
            continue;
        }
        used.insert(left_index);
        used.insert(right_index);
        merges.push(merge(&entries[left_index], &entries[right_index]));
    }
    merges
}

fn merge(left: &Entry, right: &Entry) -> Merge {
    let (a, b) = (&left.finding, &right.finding);
    let left_body = if (a.confidence - b.confidence).abs() > CONFIDENCE_GAP {
        a.confidence > b.confidence
    } else {
        a.body.chars().count() >= b.body.chars().count()
    };
    let (keep, drop) = if left_body { (left, right) } else { (right, left) };
    let severity = if severity_rank(a.severity) <= severity_rank(b.severity) { a.severity } else { b.severity };
    Merge {
        keep: keep.id.clone(),
        drop: drop.id.clone(),
        severity,
        confidence: a.confidence.max(b.confidence),
        pass_ids: [keep.pass_id.clone(), drop.pass_id.clone()],
    }
}

/// Lines between two findings' ranges; 0 when they overlap.
fn line_gap(a: &DraftFinding, b: &DraftFinding) -> u32 {
    let (a_start, a_end) = range(a);
    let (b_start, b_end) = range(b);
    if a_end < b_start {
        b_start - a_end
    } else if b_end < a_start {
        a_start - b_end
    } else {
        0
    }
}

fn range(finding: &DraftFinding) -> (u32, u32) {
    let start = finding.start_line.unwrap_or(finding.line);
    (start.min(finding.line), start.max(finding.line))
}

/// Jaccard overlap of two titles' meaningful words.
pub fn title_overlap(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(&b).count();
    let union = a.union(&b).count();
    shared as f64 / union as f64
}

fn words(title: &str) -> HashSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(&word.as_str()))
        .collect()
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
