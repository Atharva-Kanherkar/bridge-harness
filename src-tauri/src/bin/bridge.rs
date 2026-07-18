use bridge_deck_lib::learning_job;
use std::path::PathBuf;

fn usage(program: &str) -> String {
    format!(
        "usage: {program} learning run --database <bridge.db> --trigger <manual|in-app|codex:ID|claude:ID|opencode:ID> [--credential-ref <reference>]"
    )
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|value| value == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let program = args.first().map(String::as_str).unwrap_or("bridge");
    if args.get(1).map(String::as_str) != Some("learning")
        || args.get(2).map(String::as_str) != Some("run")
    {
        eprintln!("{}", usage(program));
        std::process::exit(2);
    }
    let database = value_after(&args, "--database")
        .or_else(|| std::env::var("BRIDGE_DB").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("{}", usage(program));
            std::process::exit(2);
        });
    let trigger = value_after(&args, "--trigger").unwrap_or_else(|| "manual".into());
    let credential_ref = value_after(&args, "--credential-ref");
    match learning_job::run_database(&database, &trigger, credential_ref.as_deref()) {
        Ok(run) => println!(
            "{}",
            serde_json::to_string_pretty(&run).expect("learning run should serialize")
        ),
        Err(error) => {
            eprintln!("Bridge learning run failed: {error}");
            std::process::exit(1);
        }
    }
}
