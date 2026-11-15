//! Measures how long Bridge takes to get a harness from "send" to "first
//! token", and prints one JSON line per launch.
//!
//! These tests drive real provider processes and need the machine's own
//! sign-in, so they are `#[ignore]` by default:
//!
//! ```sh
//! cargo test -p bridge-core --test launch_latency -- --ignored --nocapture
//! BENCH_HARNESS=codex BENCH_ROUNDS=3 cargo test ...
//! ```
//!
//! A cold launch is the number that matters: the first send in a new chat, or
//! the first send after a harness or model switch. Bridge records the phases it
//! observes (`spawning`, `handshake`, `session_open`) as `CoreEvent::SessionStartup`
//! events; this harness timestamps those on the event bus and reports the gap
//! from the invoke to the first assistant frame.

use bridge_core::{api, events::CoreEvent, model::Harness, BootConfig, BridgeCore};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct Launch {
    invoke_ms: f64,
    spawning_ms: Option<f64>,
    handshake_ms: Option<f64>,
    session_open_ms: Option<f64>,
    prompt_ms: Option<f64>,
    first_assistant_ms: Option<f64>,
}

fn harness_id() -> String {
    std::env::var("BENCH_HARNESS").unwrap_or_else(|_| "claude".into())
}

fn boot(dir: &std::path::Path) -> Arc<BridgeCore> {
    Arc::new(
        BridgeCore::boot(BootConfig {
            data_dir: dir.to_path_buf(),
            browser_extension_path: dir.join("no-extension"),
            events: None,
        })
        .expect("boot"),
    )
}

fn measure(core: &Arc<BridgeCore>, session_id: &str, prompt: &str) -> Launch {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut events = core.events.subscribe();
    std::thread::spawn(move || loop {
        match events.blocking_recv() {
            Ok(event) => {
                if sender.send((Instant::now(), event)).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    });
    let started = Instant::now();
    api::start_chat(core, session_id.to_owned()).expect("start_chat");
    let invoke_ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut launch = Launch {
        invoke_ms,
        spawning_ms: None,
        handshake_ms: None,
        session_open_ms: None,
        prompt_ms: None,
        first_assistant_ms: None,
    };
    // The composer sends the prompt as soon as the launch returns, exactly as
    // the app does, so the send-to-first-token number is one continuous wait.
    let sent = Instant::now();
    api::submit_input(core, session_id.to_owned(), prompt.to_owned()).expect("submit_input");
    launch.prompt_ms = Some(sent.saturating_duration_since(started).as_secs_f64() * 1000.0);
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok((at, event)) = receiver.recv_timeout(remaining.min(Duration::from_millis(500)))
        else {
            if launch.first_assistant_ms.is_some() {
                break;
            }
            continue;
        };
        let elapsed = at.saturating_duration_since(started).as_secs_f64() * 1000.0;
        match event {
            CoreEvent::SessionStartup { phase, .. } => {
                let slot = match phase.as_str() {
                    "spawning" => &mut launch.spawning_ms,
                    "handshake" => &mut launch.handshake_ms,
                    "session_open" => &mut launch.session_open_ms,
                    _ => continue,
                };
                if slot.is_none() {
                    *slot = Some(elapsed);
                }
            }
            CoreEvent::Agent(agent) => {
                let assistant = agent.kind.contains("assistant")
                    || agent.role.as_deref() == Some("assistant");
                if launch.first_assistant_ms.is_none() && assistant {
                    launch.first_assistant_ms = Some(elapsed);
                    break;
                }
            }
            _ => {}
        }
    }
    launch
}

fn wait_for_idle(core: &Arc<BridgeCore>, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        let status: String = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id=?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap_or_default();
        if status != "working" && status != "starting" {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "drives real provider processes and needs local sign-in"]
fn cold_launch_latency() {
    let rounds: usize = std::env::var("BENCH_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
        .max(1);
    let harness = harness_id();
    let model = std::env::var("BENCH_MODEL").ok();
    let fixture = tempfile::tempdir().expect("tempdir");
    let core = boot(fixture.path());
    std::thread::sleep(Duration::from_secs(2));
    let parsed = Harness::parse(&harness).expect("known harness");

    for round in 0..rounds {
        let created = api::create_chat_id(&core, &parsed, model.as_deref(), Some("bench"))
            .expect("create chat");
        let launch = measure(&core, &created.session_id, "Reply exactly BENCH_OK. Do not use tools.");
        println!(
            "{}",
            serde_json::json!({
                "round": round,
                "harness": harness,
                "invoke_ms": launch.invoke_ms.round(),
                "spawning_ms": launch.spawning_ms.map(|value| value.round()),
                "handshake_ms": launch.handshake_ms.map(|value| value.round()),
                "session_open_ms": launch.session_open_ms.map(|value| value.round()),
                "prompt_sent_ms": launch.prompt_ms.map(|value| value.round()),
                "first_assistant_ms": launch.first_assistant_ms.map(|value| value.round()),
            })
        );
        // Leave the cold path cold: stop the adapter so the next round is
        // another cold launch, the way a new chat is.
        let _ = api::stop_session(&core, created.session_id.clone());
    }
}

/// What a model or harness change costs, and what the send after it costs.
///
/// The switch command itself, then the next turn's first token: a person who
/// changes model waits for both before they see an answer.
#[test]
#[ignore = "drives real provider processes and needs local sign-in"]
fn model_switch_latency() {
    let harness = harness_id();
    let model = std::env::var("BENCH_MODEL").ok();
    let next_model = std::env::var("BENCH_NEXT_MODEL").ok();
    let fixture = tempfile::tempdir().expect("tempdir");
    let core = boot(fixture.path());
    std::thread::sleep(Duration::from_secs(2));
    let parsed = Harness::parse(&harness).expect("known harness");
    let created = api::create_chat_id(&core, &parsed, model.as_deref(), Some("bench"))
        .expect("create chat");
    let first = measure(&core, &created.session_id, "Reply exactly BENCH_OK. Do not use tools.");
    println!(
        "{}",
        serde_json::json!({
            "step": "cold",
            "invoke_ms": first.invoke_ms.round(),
            "first_assistant_ms": first.first_assistant_ms.map(|value| value.round()),
        })
    );

    // Pick a model the running build actually offers, so the switch is the
    // one a person could make in the picker.
    let next_model = next_model.or_else(|| {
        core.adapter_registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == harness)
            .and_then(|descriptor| {
                let models: Vec<String> =
                    descriptor.models.iter().map(|option| option.id.clone()).collect();
                eprintln!("offered models: {models:?}");
                models
                    .into_iter()
                    .find(|id| Some(id.as_str()) != model.as_deref())
            })
    });
    eprintln!("switching to {next_model:?}");

    let switched = Instant::now();
    wait_for_idle(&core, &created.session_id);
    api::update_chat_model(&core, &created.session_id, &parsed, next_model.as_deref(), None)
        .expect("switch model");
    let switch_ms = switched.elapsed().as_secs_f64() * 1000.0;

    let second = measure(&core, &created.session_id, "Reply exactly BENCH_OK. Do not use tools.");
    println!(
        "{}",
        serde_json::json!({
            "step": "after-switch",
            "switch_ms": switch_ms.round(),
            "invoke_ms": second.invoke_ms.round(),
            "first_assistant_ms": second.first_assistant_ms.map(|value| value.round()),
        })
    );
    let _ = api::stop_session(&core, created.session_id.clone());
}
