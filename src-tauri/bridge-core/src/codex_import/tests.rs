use super::*;

/// One real rollout (`MEASURE_FILE`), read alone. Prints aggregate numbers.
#[test]
#[ignore = "reads a real rollout; run by hand to measure"]
fn measure_one_rollout() {
    let path = PathBuf::from(std::env::var("MEASURE_FILE").expect("set MEASURE_FILE"));
    let home = home_of(&path).unwrap();
    let parsed = rollout::read_rollout(&rollout::RolloutSource { path: &path, home: &home, titles: &HashMap::new(), archived: false, label: "x".into() }).unwrap();
    let text: usize = parsed.chat.entries.iter().map(|e| e.text.as_ref().map_or(0, String::len) + e.tool.as_ref().map_or(0, |t| t.output.as_ref().map_or(0, String::len) + t.input.to_string().len())).sum();
    println!("MEASURE bytes={} entries={} kept_text_bytes={text}", std::fs::metadata(&path).unwrap().len(), parsed.chat.entries.len());
}
