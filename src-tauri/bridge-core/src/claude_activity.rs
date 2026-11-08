//! Native task activity survives assistant turns. Launcher results and task
//! results are different facts; only the latter can finish background work.
use super::{ClaudeToolCall, NormalizedEvent};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone)]
pub(super) struct ActivityState {
    tasks: HashMap<String, Task>,
}

#[derive(Debug, Clone)]
struct Task {
    id: String,
    tool_id: Option<String>,
    title: String,
    command: Option<String>,
    status: String,
    background: bool,
    ambient: bool,
    passive: bool,
    progress: Option<String>,
    output: Option<String>,
    duration_ms: Option<u64>,
    output_file: Option<String>,
}

impl Task {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            tool_id: None,
            title: "Background task".into(),
            command: None,
            status: "inProgress".into(),
            background: true,
            ambient: false,
            passive: false,
            progress: None,
            output: None,
            duration_ms: None,
            output_file: None,
        }
    }

    fn event(&self) -> NormalizedEvent {
        let mut event = NormalizedEvent::new(if self.status == "inProgress" {
            "tool.started"
        } else {
            "tool.completed"
        });
        event.item_id = Some(format!("background-task:{}", self.id));
        event.title = Some(self.title.clone());
        event.status = Some(self.status.clone());
        event.text = self.output.clone();
        event.data = json!({
            "type": "backgroundTask", "backgroundTask": true, "taskId": self.id,
            "sourceCallId": self.tool_id,
            "background": self.background && self.status == "inProgress" && (!self.ambient || self.passive),
            "passive": self.passive,
            "command": self.command, "progress": self.progress,
            "durationMs": self.duration_ms, "outputFile": self.output_file,
            "aggregatedOutput": self.output,
        });
        event
    }

    fn attach(&mut self, tool_id: Option<&str>, calls: &HashMap<String, ClaudeToolCall>) {
        if let Some(id) = tool_id {
            self.tool_id = Some(id.into());
            if let Some(call) = calls.get(id) {
                self.command = call.command.clone();
                if self.title == "Background task" {
                    self.title = call
                        .description
                        .clone()
                        .or_else(|| call.command.clone())
                        .unwrap_or_else(|| call.name.clone());
                }
            }
        }
    }
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub(super) fn normalize(
    message: &Value,
    state: &mut ActivityState,
    calls: &HashMap<String, ClaudeToolCall>,
) -> Option<Vec<NormalizedEvent>> {
    let kind = text(message, "type")?;
    if kind == "tool_progress" {
        let id = text(message, "tool_use_id")?;
        let task_id = text(message, "task_id").map(str::to_owned).or_else(|| {
            state
                .tasks
                .iter()
                .find(|(_, task)| task.tool_id.as_deref() == Some(id))
                .map(|(id, _)| id.clone())
        });
        if let Some(task) = task_id.and_then(|id| state.tasks.get_mut(&id)) {
            task.duration_ms = message
                .get("elapsed_time_seconds")
                .and_then(Value::as_f64)
                .map(|s| (s.max(0.0) * 1000.0) as u64);
            task.progress = text(message, "tool_name").map(|s| format!("Using {s}"));
            let mut event = task.event();
            event.kind = "tool.progress".into();
            return Some(if task.ambient && !task.passive {
                vec![]
            } else {
                vec![event]
            });
        }
        let mut event = NormalizedEvent::new("tool.progress");
        event.item_id = Some(id.into());
        event.status = Some("inProgress".into());
        // Do not overwrite the command's title with the tool's generic name.
        event.title =
            (!calls.contains_key(id)).then(|| text(message, "tool_name").unwrap_or("Tool").into());
        event.data = json!({"durationMs": message.get("elapsed_time_seconds").and_then(Value::as_f64).map(|s| (s.max(0.0) * 1000.0) as u64)});
        return Some(vec![event]);
    }
    if kind != "system" {
        return None;
    }
    let subtype = text(message, "subtype")?;
    if subtype == "background_tasks_changed" {
        let members = message.get("tasks")?.as_array()?;
        let present: HashSet<_> = members.iter().filter_map(|v| text(v, "task_id")).collect();
        let mut events = Vec::new();
        for task in state.tasks.values_mut() {
            if task.status == "inProgress" && task.background && !present.contains(task.id.as_str())
            {
                task.status = "unknown".into();
                task.background = false;
                task.progress = Some("No longer reported as running. Outcome unavailable.".into());
                if !task.ambient || task.passive {
                    events.push(task.event());
                }
            }
        }
        for member in members {
            let Some(id) = text(member, "task_id") else {
                continue;
            };
            let existed = state.tasks.contains_key(id);
            let task = state
                .tasks
                .entry(id.into())
                .or_insert_with(|| Task::new(id));
            // A late level snapshot must not revive a task whose terminal
            // notification already arrived. The bookends and level are unordered.
            if matches!(task.status.as_str(), "completed" | "failed" | "stopped") {
                continue;
            }
            if task.status == "unknown" {
                task.progress = None;
            }
            task.status = "inProgress".into();
            if let Some(title) = text(member, "description") {
                task.title = title.into();
            }
            task.background = true;
            let was_visible = existed && (!task.ambient || task.passive);
            task.ambient = member
                .get("ambient")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if task.ambient && !task.passive && was_visible {
                let mut hidden = task.event();
                hidden.status = Some("unknown".into());
                events.push(hidden);
            } else if !task.ambient || task.passive {
                events.push(task.event());
            }
        }
        events.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        return Some(events);
    }
    if !matches!(
        subtype,
        "task_started" | "task_progress" | "task_updated" | "task_notification"
    ) {
        return None;
    }
    let Some(id) = text(message, "task_id") else {
        return Some(vec![]);
    };
    // A resumed task can reuse its id. Its new run must not display the old
    // run's report, command, progress or duration as current work.
    if subtype == "task_started"
        && state
            .tasks
            .get(id)
            .is_some_and(|task| task.status != "inProgress")
    {
        state.tasks.insert(id.into(), Task::new(id));
    }
    let task = state
        .tasks
        .entry(id.into())
        .or_insert_with(|| Task::new(id));
    task.attach(text(message, "tool_use_id"), calls);
    if let Some(title) = text(message, "description") {
        task.title = title.into();
    }
    if message.get("ambient").and_then(Value::as_bool) == Some(true)
        || message.get("skip_transcript").and_then(Value::as_bool) == Some(true)
    {
        task.ambient = true;
    }
    if let Some(background) = message.get("is_backgrounded").and_then(Value::as_bool) {
        task.background = background;
    }
    if let Some(ms) = message
        .pointer("/usage/duration_ms")
        .and_then(Value::as_u64)
    {
        task.duration_ms = Some(ms);
    }
    match subtype {
        "task_started" => {
            task.status = "inProgress".into();
        }
        "task_progress" => {
            task.progress = text(message, "summary")
                .map(str::to_owned)
                .or_else(|| text(message, "last_tool_name").map(|s| format!("Using {s}")));
        }
        "task_updated" => {
            let patch = &message["patch"];
            if let Some(title) = text(patch, "description") {
                task.title = title.into();
            }
            if let Some(background) = patch.get("is_backgrounded").and_then(Value::as_bool) {
                task.background = background;
            }
            if let Some(status) = text(patch, "status") {
                task.status = status_name(status).into();
            }
            if let Some(error) = text(patch, "error") {
                task.output = Some(error.into());
            }
        }
        "task_notification" => {
            task.status = status_name(text(message, "status").unwrap_or("unknown")).into();
            task.output = text(message, "summary").map(str::to_owned);
            task.progress = task.output.clone();
            task.output_file = text(message, "output_file").map(str::to_owned);
        }
        _ => {}
    }
    Some(if task.ambient && !task.passive {
        vec![]
    } else {
        vec![task.event()]
    })
}

fn status_name(status: &str) -> &str {
    match status {
        "running" | "pending" => "inProgress",
        "killed" | "stopped" => "stopped",
        other => other,
    }
}

/// Uses the SDK's structured tool result, never its model-facing prose.
pub(super) fn launched(
    message: &Value,
    tool_id: &str,
    call: Option<&ClaudeToolCall>,
    state: &mut ActivityState,
) -> (bool, Vec<NormalizedEvent>) {
    let result = &message["tool_use_result"];
    let id = text(result, "backgroundTaskId").or_else(|| {
        let native_async = call.is_some_and(|c| c.name == "Monitor")
            || matches!(
                text(result, "status"),
                Some("async_launched" | "remote_launched")
            );
        native_async
            .then(|| text(result, "taskId").or_else(|| text(result, "agentId")))
            .flatten()
    });
    let id = id.map(str::to_owned).or_else(|| {
        state
            .tasks
            .values()
            .find(|task| {
                task.tool_id.as_deref() == Some(tool_id)
                    && task.background
                    && task.status == "inProgress"
            })
            .map(|task| task.id.clone())
    });
    let Some(id) = id else {
        return (false, vec![]);
    };
    let task = state
        .tasks
        .entry(id.clone())
        .or_insert_with(|| Task::new(&id));
    task.tool_id = Some(tool_id.into());
    task.output_file = text(result, "outputFile")
        .map(str::to_owned)
        .or(task.output_file.take());
    if let Some(call) = call {
        task.command = call.command.clone();
        if task.title == "Background task" {
            task.title = call
                .description
                .clone()
                .or_else(|| call.command.clone())
                .unwrap_or_else(|| call.name.clone());
        }
        // A watch does not mean the model or tests are busy. Its delivered
        // notifications remain inspectable through the synthetic-message path.
        if call.name == "Monitor" {
            task.passive = true;
        }
    }
    (
        true,
        if task.ambient && !task.passive {
            vec![]
        } else {
            vec![task.event()]
        },
    )
}

pub(super) fn synthetic_notification(message: &Value) -> Option<NormalizedEvent> {
    if message.pointer("/origin/kind").and_then(Value::as_str) != Some("task-notification") {
        return None;
    }
    let content = &message["message"]["content"];
    let output = content.as_str().map(str::to_owned).unwrap_or_else(|| {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| text(block, "text"))
            .collect::<Vec<_>>()
            .join("\n")
    });
    if output.is_empty() {
        return None;
    }
    // Monitor delivers a documented task-notification envelope as text. Use
    // only its output fields for presentation; never infer task status here.
    let body = ["event", "summary"]
        .into_iter()
        .find_map(|tag| {
            let opening = format!("<{tag}>");
            let closing = format!("</{tag}>");
            let start = output.find(&opening)? + opening.len();
            let end = output[start..].find(&closing)? + start;
            Some(output[start..end].trim().to_owned())
        })
        .unwrap_or_else(|| output.clone());
    let mut event = NormalizedEvent::new("tool.completed");
    event.item_id = text(message, "uuid").map(|id| format!("task-notice:{id}"));
    event.title = Some("Background update".into());
    event.status = Some("completed".into());
    event.text = Some(body.clone());
    event.data = json!({"kind": "other", "aggregatedOutput": body, "progress": body.lines().last(), "rawNotification": output});
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::super::{normalize_claude_message_with_state as normalize, ClaudeStreamState};
    use super::*;

    fn call(state: &mut ClaudeStreamState, name: &str) {
        normalize(
            &json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"launcher","name":name,"input":{"command":"cargo test","description":"Rust tests"}}]}}),
            state,
        );
    }
    fn task(subtype: &str) -> Value {
        json!({"type":"system","subtype":subtype,"task_id":"tests","tool_use_id":"launcher","description":"Rust tests","is_backgrounded":true})
    }

    #[test]
    fn native_processes_keep_their_lifecycle_after_the_call_yields() {
        let start = super::super::normalize_codex_message(
            &json!({"method":"item/started","params":{"item":{"id":"exec","type":"commandExecution","command":"cargo test","processId":"123","status":"inProgress"}}}),
        );
        assert_eq!(start[0].data["background"], true);
        let done = super::super::normalize_codex_message(
            &json!({"method":"item/completed","params":{"item":{"id":"exec","type":"commandExecution","command":"cargo test","processId":"123","status":"completed","exitCode":0}}}),
        );
        assert_eq!(done[0].data["background"], false);
        assert_eq!(done[0].data["backgroundTask"], true);
    }

    #[test]
    fn native_tasks_keep_identity_command_progress_and_real_outcome_across_turns() {
        let mut state = ClaudeStreamState::default();
        call(&mut state, "Bash");
        let start = normalize(&task("task_started"), &mut state);
        assert_eq!(start[0].data["background"], true);
        assert_eq!(start[0].data["command"], "cargo test");
        let ack = normalize(
            &json!({"type":"user","tool_use_result":{"backgroundTaskId":"tests"},"message":{"content":[{"type":"tool_result","tool_use_id":"launcher","content":"launched"}]}}),
            &mut state,
        );
        assert_eq!(ack[0].status.as_deref(), Some("backgrounded"));
        assert_eq!(ack[1].item_id, start[0].item_id);
        normalize(
            &json!({"type":"result","subtype":"success","result":"Tests are running"}),
            &mut state,
        );
        let mut progress = task("task_progress");
        progress["summary"] = json!("Running bridge-core tests");
        progress["usage"] = json!({"duration_ms":3000});
        let update = normalize(&progress, &mut state);
        assert_eq!(update[0].item_id, start[0].item_id);
        assert_eq!(update[0].data["progress"], "Running bridge-core tests");
        assert_eq!(update[0].data["command"], "cargo test");
        let mut done = task("task_notification");
        done["status"] = json!("failed");
        done["summary"] = json!("test example FAILED");
        done["output_file"] = json!("/tmp/tests.output");
        let finish = normalize(&done, &mut state);
        assert_eq!(finish[0].status.as_deref(), Some("failed"));
        assert_eq!(finish[0].data["background"], false);
        assert_eq!(finish[0].text.as_deref(), Some("test example FAILED"));
        assert_eq!(finish[0].item_id, start[0].item_id);
    }

    #[test]
    fn membership_and_bookends_can_arrive_in_either_order_without_duplicate_identity() {
        for level_first in [true, false] {
            let mut state = ClaudeStreamState::default();
            call(&mut state, "Bash");
            let level = json!({"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tests","task_type":"local_bash","description":"Rust tests"}]});
            let started = task("task_started");
            let frames = if level_first {
                [&level, &started]
            } else {
                [&started, &level]
            };
            let a = normalize(frames[0], &mut state);
            let b = normalize(frames[1], &mut state);
            assert_eq!(a[0].item_id, b[0].item_id);
            let missing = normalize(
                &json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),
                &mut state,
            );
            assert_eq!(missing[0].status.as_deref(), Some("unknown"));
            let mut done = task("task_notification");
            done["status"] = json!("completed");
            assert_eq!(
                normalize(&done, &mut state)[0].status.as_deref(),
                Some("completed")
            );
            assert!(
                normalize(&level, &mut state).is_empty(),
                "a stale level must not revive a completed task"
            );
        }
    }

    #[test]
    fn a_resumed_task_does_not_reuse_the_previous_runs_output() {
        let mut state = ClaudeStreamState::default();
        call(&mut state, "Bash");
        normalize(&task("task_started"), &mut state);
        let mut done = task("task_notification");
        done["status"] = json!("completed");
        done["summary"] = json!("Old test result");
        normalize(&done, &mut state);
        normalize(&json!({"type":"result","subtype":"success"}), &mut state);
        let restarted = normalize(&task("task_started"), &mut state);
        assert_eq!(restarted[0].status.as_deref(), Some("inProgress"));
        assert!(restarted[0].text.is_none());
        assert!(restarted[0].data["aggregatedOutput"].is_null());
        assert!(restarted[0].data["progress"].is_null());
        assert!(restarted[0].data["command"].is_null());
    }

    #[test]
    fn foreground_to_background_and_stop_are_explicit() {
        let mut state = ClaudeStreamState::default();
        let mut start = task("task_started");
        start["is_backgrounded"] = json!(false);
        assert_eq!(normalize(&start, &mut state)[0].data["background"], false);
        let update = json!({"type":"system","subtype":"task_updated","task_id":"tests","patch":{"is_backgrounded":true,"description":"Verifying Rust"}});
        assert_eq!(normalize(&update, &mut state)[0].data["background"], true);
        let mut stop = task("task_notification");
        stop["status"] = json!("stopped");
        assert_eq!(
            normalize(&stop, &mut state)[0].status.as_deref(),
            Some("stopped")
        );
    }

    #[test]
    fn tasks_without_tool_ids_are_visible_but_housekeeping_and_watchers_are_not_busy() {
        let mut state = ClaudeStreamState::default();
        let standalone = json!({"type":"system","subtype":"task_started","task_id":"orphan","description":"External verification"});
        assert_eq!(
            normalize(&standalone, &mut state)[0].title.as_deref(),
            Some("External verification")
        );
        for field in ["ambient", "skip_transcript"] {
            let mut frame = task("task_started");
            frame[field] = json!(true);
            assert!(normalize(&frame, &mut state).is_empty());
        }
        call(&mut state, "Monitor");
        let ack = normalize(
            &json!({"type":"user","tool_use_result":{"taskId":"watch","timeoutMs":60000},"message":{"content":[{"type":"tool_result","tool_use_id":"launcher","content":"started"}]}}),
            &mut state,
        );
        assert_eq!(ack.len(), 2);
        assert_eq!(ack[0].status.as_deref(), Some("backgrounded"));
        assert_eq!(ack[1].data["passive"], true);
    }

    #[test]
    fn tool_progress_updates_existing_call_without_losing_its_command_title() {
        let mut state = ClaudeStreamState::default();
        call(&mut state, "Bash");
        let events = normalize(
            &json!({"type":"tool_progress","tool_use_id":"launcher","tool_name":"Bash","elapsed_time_seconds":7}),
            &mut state,
        );
        assert_eq!(events[0].kind, "tool.progress");
        assert_eq!(events[0].item_id.as_deref(), Some("launcher"));
        assert!(events[0].title.is_none());
        assert_eq!(events[0].data["durationMs"], 7000);
    }

    #[test]
    fn synthetic_monitor_notices_are_visible_without_echoing_human_messages() {
        let mut state = ClaudeStreamState::default();
        let mut frame = json!({"type":"user","uuid":"notice","origin":{"kind":"task-notification"},"message":{"content":[{"type":"text","text":"bridge-core: 402 passed"}]}});
        let events = normalize(&frame, &mut state);
        assert_eq!(events[0].kind, "tool.completed");
        assert_eq!(events[0].title.as_deref(), Some("Background update"));
        assert_eq!(events[0].text.as_deref(), Some("bridge-core: 402 passed"));
        let envelope = "<task-notification><task-id>watch</task-id><summary>Monitor event</summary><event>Running bridge-core tests\ntest example ... ok</event></task-notification>";
        frame["message"]["content"] = json!(envelope);
        let delivered = normalize(&frame, &mut state);
        assert_eq!(
            delivered[0].text.as_deref(),
            Some("Running bridge-core tests\ntest example ... ok")
        );
        assert_eq!(delivered[0].data["rawNotification"], envelope);
        assert_eq!(delivered[0].data["progress"], "test example ... ok");
        frame["origin"] = json!({"kind":"human"});
        assert!(normalize(&frame, &mut state).is_empty());
    }

    #[test]
    fn background_snapshots_and_progress_are_durable_with_the_same_identity() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported')",[]).unwrap();
        let mut state = ClaudeStreamState::default();
        call(&mut state, "Bash");
        let mut progress = task("task_progress");
        progress["summary"] = json!("Running test binary");
        for frame in [task("task_started"), progress] {
            for event in normalize(&frame, &mut state) {
                crate::store::session_event(&db, "s", &event, &json!({"adapter":"claude"}))
                    .unwrap();
            }
        }
        let rows = crate::store::session_events_tail(&db, "s", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].item_id, rows[1].item_id);
        assert_eq!(rows[1].data["progress"], "Running test binary");
    }
}
