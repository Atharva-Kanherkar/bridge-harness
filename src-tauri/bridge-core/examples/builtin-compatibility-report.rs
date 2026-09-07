fn main() {
    let report = bridge_core::builtin_compatibility::compatibility_report();
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("built-in compatibility report serializes")
    );
}
