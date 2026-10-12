use crate::{
    adapters, binary,
    frame_queue::{self, FrameReceiver, QueueBudget, QueueMetrics},
    BridgeError,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

const MAX_RECORD: usize = 8 * 1024 * 1024;
type Reply = Result<Value, String>;
struct WriteRequest {
    bytes: Vec<u8>,
    reply: mpsc::SyncSender<Result<(), String>>,
}
struct Pending {
    command: String,
    reply: Option<mpsc::SyncSender<Reply>>,
}

/// Split only on LF; do not expose invalid payloads in parse diagnostics.
fn record(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader
            .fill_buf()
            .map_err(|_| "Pi stdout could not be read")?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err("Pi stdout ended inside a JSONL record".into())
            };
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > MAX_RECORD {
            return Err("Pi RPC record exceeds the 8 MiB limit".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if end.is_some() {
            break;
        }
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Pi emitted malformed JSONL; update or repair the Pi CLI")?;
    if !value.is_object() || value.get("type").and_then(Value::as_str).is_none() {
        return Err("Pi RPC record is not a typed JSON object".into());
    }
    Ok(Some(value))
}

pub(super) struct Client {
    writer: Mutex<Option<mpsc::SyncSender<WriteRequest>>>,
    pending: Mutex<HashMap<String, Pending>>,
    sequence: AtomicU64,
    pub stopped: AtomicBool,
    pub session: Mutex<String>,
    pub permissions: Mutex<HashMap<String, String>>,
    pub ready: AtomicBool,
    pid: u32,
}
impl Client {
    pub fn write(&self, value: &Value) -> Result<(), BridgeError> {
        self.write_deadline(value, super::COMMAND_TIMEOUT)
    }
    fn write_deadline(&self, value: &Value, timeout: Duration) -> Result<(), BridgeError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(BridgeError::Invalid("Pi RPC process has stopped".into()));
        }
        let deadline = std::time::Instant::now() + timeout;
        let mut bytes = serde_json::to_vec(value)
            .map_err(|_| BridgeError::Invalid("Could not encode Pi command".into()))?;
        if bytes.len() >= MAX_RECORD {
            return Err(BridgeError::Invalid(
                "Pi command exceeds the 8 MiB limit".into(),
            ));
        }
        bytes.push(b'\n');
        let writer = self
            .writer
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| BridgeError::Invalid("Pi stdin is closed".into()))?;
        let (reply, receiver) = mpsc::sync_channel(1);
        let mut request = WriteRequest { bytes, reply };
        let outcome = loop {
            match writer.try_send(request) {
                Ok(()) => {
                    break receiver
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .ok()
                        .and_then(Result::ok)
                        .ok_or(())
                }
                Err(mpsc::TrySendError::Full(returned)) if std::time::Instant::now() < deadline => {
                    request = returned;
                    thread::sleep(Duration::from_millis(2));
                }
                _ => break Err(()),
            }
        };
        if outcome.is_err() {
            self.fail("Pi stopped accepting stdin commands");
            self.close();
            adapters::terminate_process_group(self.pid);
            return Err(BridgeError::Invalid(
                "Pi stdin failed or exceeded its write deadline; the process was stopped".into(),
            ));
        }
        Ok(())
    }
    pub fn command(&self, mut value: Value, timeout: Duration) -> Result<Value, BridgeError> {
        let deadline = std::time::Instant::now() + timeout;
        let command = value["type"].as_str().unwrap_or("unknown").to_owned();
        let id = format!("bridge-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        value["id"] = json!(id);
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                command: command.clone(),
                reply: Some(sender),
            },
        );
        let outcome=self.write_deadline(&value,timeout).and_then(|_| receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).map_err(|_| BridgeError::Invalid(format!("Pi {command} did not respond before the deadline; check the Pi CLI and provider setup")))?.map_err(BridgeError::Invalid));
        self.pending.lock().unwrap().remove(&id);
        outcome
    }
    pub fn command_async(&self, mut value: Value) -> Result<(), BridgeError> {
        let command = value["type"].as_str().unwrap_or("unknown").to_owned();
        let id = format!("bridge-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        value["id"] = json!(id);
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                command,
                reply: None,
            },
        );
        let result = self.write(&value);
        if result.is_err() {
            self.pending.lock().unwrap().remove(&id);
        }
        result
    }
    fn fail(&self, reason: &str) {
        self.stopped.store(true, Ordering::Release);
        for (_, pending) in self.pending.lock().unwrap().drain() {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err(reason.into()));
            }
        }
    }
    pub fn close(&self) {
        self.writer.lock().unwrap().take();
    }
}

pub(super) struct Process {
    pub child: Child,
    pub client: Arc<Client>,
    pub metrics: QueueMetrics,
    pub diagnostic: Arc<AtomicBool>,
    reaped: bool,
}
impl Process {
    pub fn spawn(
        executable: &Path,
        args: &[String],
        cwd: &str,
    ) -> Result<(Self, Reader), BridgeError> {
        let mut command = adapters::supervised_command(executable, args);
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        adapters::configure_process_group(&mut command);
        binary::hydrate_command_path(&mut command);
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (writer, requests) = mpsc::sync_channel::<WriteRequest>(1);
        thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let result = stdin
                    .write_all(&request.bytes)
                    .and_then(|_| stdin.flush())
                    .map_err(|_| "Pi stdin write failed".to_owned());
                let failed = result.is_err();
                let _ = request.reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let client = Arc::new(Client {
            writer: Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            session: Mutex::new(String::new()),
            permissions: Mutex::new(HashMap::new()),
            ready: AtomicBool::new(false),
            pid: child.id(),
        });
        let (sender, receiver, metrics) = frame_queue::bounded_frame_queue(QueueBudget::default());
        let diagnostic = Arc::new(AtomicBool::new(false));
        let stderr = child.stderr.take().expect("piped stderr");
        let seen = diagnostic.clone();
        thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = [0; 4096];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                seen.store(true, Ordering::Relaxed);
            }
        });
        let stdout = child.stdout.take().expect("piped stdout");
        let pid = child.id();
        let shared = client.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut value = match record(&mut reader) {
                    Ok(Some(value)) => value,
                    result => {
                        let reason = match result { Err(reason) => reason, _ => "Pi RPC process exited unexpectedly; run pi directly to check provider setup".into() };
                        let intentional = shared.stopped.load(Ordering::Acquire);
                        shared.fail(&reason);
                        if !intentional {
                            adapters::terminate_process_group(pid);
                        }
                        if !intentional {
                            let frame = json!({"piSession":*shared.session.lock().unwrap(),"event":{"type":"bridge_error","message":reason}});
                            let _ = sender.send_durable(format!("{frame}\n"));
                        }
                        break;
                    }
                };
                if value["type"] == "response" {
                    let id = value["id"].as_str().unwrap_or("");
                    let pending = shared.pending.lock().unwrap().remove(id);
                    if let Some(pending) = pending {
                        let response = if value["command"] != pending.command
                            || !value["success"].is_boolean()
                        {
                            Err(format!(
                                "Pi returned an incompatible response for {}",
                                pending.command
                            ))
                        } else if value["success"] != true {
                            // Provider errors may contain headers, tokens or credential values.
                            Err(format!(
                                "Pi rejected {}; check its CLI/provider configuration",
                                pending.command
                            ))
                        } else {
                            Ok(value.get("data").cloned().unwrap_or(Value::Null))
                        };
                        let handled = pending.command == "prompt"
                            && value.pointer("/data/disposition").and_then(Value::as_str)
                                == Some("handled")
                            && response.is_ok();
                        if let Some(reply) = pending.reply {
                            let _ = reply.send(response);
                            if !handled {
                                continue;
                            }
                            value = json!({"type":"bridge_handled"});
                        } else if response.is_err() {
                            value = json!({"type":"bridge_compaction_error"});
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    } // late/duplicate responses cannot satisfy another command
                }
                if value["type"] == "extension_ui_request" {
                    if value["method"] == "notify" && value["message"] == super::READY {
                        shared.ready.store(true, Ordering::Release);
                        continue;
                    }
                    if let Some(id) = value["id"].as_str() {
                        let owned = value["title"]
                            .as_str()
                            .and_then(|title| serde_json::from_str::<Value>(title).ok())
                            .filter(|title| title["bridgePermission"] == 1);
                        if value["method"] == "confirm" && owned.is_some() {
                            shared
                                .permissions
                                .lock()
                                .unwrap()
                                .insert(id.to_owned(), value["title"].as_str().unwrap().to_owned());
                        } else {
                            let _ = shared.write(
                                &json!({"type":"extension_ui_response","id":id,"cancelled":true}),
                            );
                            continue;
                        }
                    }
                }
                if value["type"] == "agent_settled" {
                    shared.permissions.lock().unwrap().clear();
                }
                if value["type"] == "tool_execution_end" {
                    let tool_id = value["toolCallId"].as_str().unwrap_or("");
                    shared.permissions.lock().unwrap().retain(|_, title| {
                        serde_json::from_str::<Value>(title)
                            .ok()
                            .is_some_and(|title| title["toolCallId"] != tool_id)
                    });
                }
                let transient = value["type"] == "message_update"
                    && matches!(
                        value
                            .pointer("/assistantMessageEvent/type")
                            .and_then(Value::as_str),
                        Some("text_delta" | "thinking_delta" | "toolcall_delta")
                    )
                    || value["type"] == "tool_execution_update";
                let frame = json!({"piSession":*shared.session.lock().unwrap(),"event":value});
                let frame = format!("{frame}\n");
                let sent = if transient {
                    sender.send_transient(frame).map(|_| ())
                } else {
                    sender.send_durable(frame)
                };
                if sent.is_err() {
                    break;
                }
            }
        });
        Ok((
            Self {
                child,
                client,
                metrics,
                diagnostic,
                reaped: false,
            },
            Reader {
                receiver,
                buffer: Vec::new(),
                position: 0,
            },
        ))
    }
    pub fn stop(&mut self) {
        if self.reaped {
            return;
        }
        self.reaped = true;
        self.client.stopped.store(true, Ordering::Release);
        self.client.close();
        // Closing stdin is Pi's graceful shutdown contract; bound it, then reap
        // the entire supervised group, including any active tool descendants.
        for _ in 0..10 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.client.fail("Pi session was stopped");
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) struct Reader {
    receiver: FrameReceiver,
    buffer: Vec<u8>,
    position: usize,
}
impl Read for Reader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let source = self.fill_buf()?;
        let count = source.len().min(output.len());
        output[..count].copy_from_slice(&source[..count]);
        self.consume(count);
        Ok(count)
    }
}
impl BufRead for Reader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.position >= self.buffer.len() {
            self.buffer = self.receiver.recv().unwrap_or_default().into_bytes();
            self.position = 0;
        }
        Ok(&self.buffer[self.position..])
    }
    fn consume(&mut self, count: usize) {
        self.position = (self.position + count).min(self.buffer.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_lf_framing_preserves_unicode_and_accepts_crlf() {
        let mut reader = BufReader::with_capacity(
            3,
            std::io::Cursor::new(
                "{\"type\":\"test\",\"text\":\"a\u{2028}b\u{2029}c\"}\r\n{\"type\":\"next\"}\n",
            ),
        );
        assert_eq!(
            record(&mut reader).unwrap().unwrap()["text"],
            "a\u{2028}b\u{2029}c"
        );
        assert_eq!(record(&mut reader).unwrap().unwrap()["type"], "next");
        assert_eq!(record(&mut reader).unwrap(), None);
    }
    #[test]
    fn malformed_oversized_and_truncated_records_do_not_echo_payloads() {
        for input in ["SECRET\n", "[]\n", "{}\n", "{\"type\":\"private\"}"] {
            let error = record(&mut std::io::Cursor::new(input)).unwrap_err();
            assert!(!error.contains("SECRET"));
            assert!(!error.contains("private"));
        }
        assert!(
            record(&mut std::io::Cursor::new(vec![b'a'; MAX_RECORD + 1]))
                .unwrap_err()
                .contains("limit")
        );
    }
}
