pub fn context_percent(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    for marker in ["context:", "context ", "context left:"] {
        if let Some(start) = lower.rfind(marker) {
            let tail = &lower[start + marker.len()..];
            for token in tail.split_whitespace().take(5) {
                let number = token
                    .trim_matches(|c: char| !c.is_ascii_digit())
                    .parse::<i64>()
                    .ok();
                if number.is_some_and(|n| (0..=100).contains(&n)) && token.contains('%') {
                    return number;
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_reported_context() {
        assert_eq!(
            context_percent("model codex · context: 63% remaining"),
            Some(63)
        );
    }
    #[test]
    fn ignores_unlabeled_percent() {
        assert_eq!(context_percent("tests 98% passed"), None);
    }
}
