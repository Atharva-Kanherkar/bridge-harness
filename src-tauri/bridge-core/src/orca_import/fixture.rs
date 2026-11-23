//! An Orca data folder for tests, on the real schemas Orca 1.4.271 wrote
//! (`testing/fixtures/import/orca/1.4.271/`). Rows are shaped as Orca writes
//! them. Every place Bridge must never read holds a canary: profile
//! documents other than the allowed ones, record fields Bridge does not
//! select, a worker's environment, and the secret files themselves.

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const CANARY: &str = "CANARY-orca-secret";
pub const VERSION: &str = "1.4.271";

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/import/orca").join(VERSION)
}

/// `agent-session-domain`, `orchestration` or `profile-state`.
pub fn schema(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(format!("{name}.sql"))).unwrap()
}

fn create(path: &Path, name: &str) -> Connection {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    conn.execute_batch(&schema(name)).unwrap();
    conn
}

/// The native session a chat's last handle names.
#[derive(Debug, Clone, Copy)]
pub enum Handle<'a> {
    Claude(&'a str),
    Codex(&'a str),
    Acp(&'a str, &'a str),
}

impl Handle<'_> {
    fn json(self) -> Value {
        match self {
            Self::Claude(id) => json!({ "provider": "claude", "sessionId": id, "leafUuid": "leaf" }),
            Self::Codex(id) => json!({ "provider": "codex", "threadId": id }),
            Self::Acp(agent, id) => json!({ "transport": "acp", "agent": agent, "nativeId": id }),
        }
    }
}

/// A chat as a test describes it.
#[derive(Debug, Clone)]
pub struct ChatRow<'a> {
    pub id: &'a str,
    pub agent: &'a str,
    /// `location.workspaceId`: a worktree id, `folder:<id>`, or anything.
    pub workspace: String,
    pub launch: Option<&'a Path>,
    pub host: &'a str,
    pub title: Option<&'a str>,
    pub renamed: bool,
    /// Older links first; the last is the session.
    pub handles: Vec<Handle<'a>>,
    pub account_home: Option<(&'a str, &'a Path)>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

impl Default for ChatRow<'_> {
    fn default() -> Self {
        Self {
            id: "",
            agent: "claude",
            workspace: String::new(),
            launch: None,
            host: "local",
            title: None,
            renamed: false,
            handles: Vec::new(),
            account_home: None,
            created_ms: 1_795_419_158_665,
            updated_ms: 1_795_419_465_708,
        }
    }
}

/// `<repo id>::<path>`.
pub fn worktree_id(repo: &str, path: &Path) -> String {
    format!("{repo}::{}", path.to_string_lossy())
}

pub struct Fixture {
    pub base: PathBuf,
    pub chats: Connection,
    pub runs: Connection,
}

impl Fixture {
    /// An Orca data folder at `base`, with the default profile.
    pub fn at(base: &Path) -> Self {
        let chats = create(&base.join(super::store::CHAT_DB), "agent-session-domain");
        let runs = create(&base.join(super::store::ORCHESTRATION_DB), "orchestration");
        let fixture = Self { base: base.to_path_buf(), chats, runs };
        fixture.index(&["local-default"]);
        fixture.profile("local-default");
        fixture.secret_files();
        fixture
    }

    pub fn index(&self, profiles: &[&str]) {
        let profiles: Vec<Value> = profiles
            .iter()
            .map(|id| json!({ "id": id, "name": "Personal", "kind": "cloud-linked", "cloud": { "email": CANARY } }))
            .collect();
        let index = json!({ "schemaVersion": 1, "activeProfileId": "local-default", "profiles": profiles });
        std::fs::write(self.base.join(super::store::PROFILE_INDEX), index.to_string()).unwrap();
    }

    pub fn profile_path(&self, profile: &str) -> PathBuf {
        self.base.join("profiles").join(profile).join(super::store::PROFILE_DB)
    }

    /// A profile's state, with canaries in documents Bridge must not read.
    pub fn profile(&self, profile: &str) -> Connection {
        let path = self.profile_path(profile);
        if path.is_file() {
            return Connection::open(path).unwrap();
        }
        let conn = create(&path, "profile-state");
        let document = |domain: &str, payload: Value| {
            conn.execute(
                "INSERT OR REPLACE INTO profile_state_documents(domain,payload,domain_version,revision,updated_at,content_hash) VALUES(?1,?2,1,1,0,'h')",
                params![domain, payload.to_string()],
            )
            .unwrap();
        };
        document("schemaVersion", json!(1));
        document("settings", json!({ "opencodeSessionCookie": CANARY, "agentDefaultEnv": { "ANTHROPIC_API_KEY": CANARY } }));
        document("sshTargets", json!([{ "host": CANARY, "password": CANARY }]));
        document("workspaceSession", json!({ "tabsByWorktree": { "x": [{ "title": CANARY }] } }));
        document("repos", json!([]));
        document("worktreeMeta", json!({}));
        conn
    }

    /// Files and folders Bridge never opens, each holding the canary and
    /// made unreadable.
    fn secret_files(&self) {
        use std::os::unix::fs::PermissionsExt;
        let plant = |path: PathBuf| {
            std::fs::write(&path, CANARY).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        };
        let profile = self.base.join("profiles/local-default");
        for name in super::store::DENIED_FILES {
            let at = if matches!(*name, "account-session.json.enc" | "browser-passwords") { profile.join(name) } else { self.base.join(name) };
            if name.contains('.') {
                plant(at);
            } else {
                std::fs::create_dir_all(&at).unwrap();
                plant(at.join("secret.json"));
            }
        }
    }

    pub fn set_document(&self, profile: &str, domain: &str, payload: &Value, revision: i64) {
        self.profile(profile)
            .execute(
                "INSERT OR REPLACE INTO profile_state_documents(domain,payload,domain_version,revision,updated_at,content_hash) VALUES(?1,?2,1,?3,0,'h')",
                params![domain, payload.to_string(), revision],
            )
            .unwrap();
    }

    /// Repositories: `(id, path)`.
    pub fn repos(&self, profile: &str, repos: &[(&str, &Path)]) {
        let rows: Vec<Value> = repos
            .iter()
            .map(|(id, path)| {
                json!({
                    "id": id, "path": path, "displayName": path.file_name().unwrap().to_string_lossy(), "addedAt": 1,
                    "kind": "git", "hookSettings": { "mode": "auto", "scripts": { "setup": "", "archive": "" } }
                })
            })
            .collect();
        self.set_document(profile, "repos", &Value::Array(rows), 1);
    }

    /// `worktreeMeta` as one document: `(id, display name, archived)`.
    pub fn worktree_meta(&self, profile: &str, rows: &[(String, &str, bool)]) {
        let mut map = serde_json::Map::new();
        for (id, name, archived) in rows {
            let mut meta = json!({ "instanceId": "i", "displayName": name, "comment": CANARY, "lastActivityAt": 1 });
            if *archived {
                meta["isArchived"] = json!(true);
            }
            map.insert(id.clone(), meta);
        }
        self.set_document(profile, "worktreeMeta", &Value::Object(map), 7);
    }

    /// `worktreeMeta` split into parts, current when `revision` is the
    /// document's.
    pub fn worktree_parts(&self, profile: &str, rows: &[(String, &str, bool)], revision: i64) {
        let conn = self.profile(profile);
        conn.execute(
            "INSERT OR REPLACE INTO profile_state_document_parts_meta(domain,domain_version,revision,updated_at,content_hash,legacy_revision) VALUES('worktreeMeta',1,1,0,'h',?1)",
            params![revision],
        )
        .unwrap();
        for (ordinal, (id, name, archived)) in rows.iter().enumerate() {
            let meta = json!({ "instanceId": "i", "displayName": name, "isArchived": archived });
            conn.execute(
                "INSERT OR REPLACE INTO profile_state_document_parts(domain,part_key,ordinal,payload,content_hash) VALUES('worktreeMeta',?1,?2,?3,'h')",
                params![json!([id]).to_string(), ordinal as i64 * 1024, meta.to_string()],
            )
            .unwrap();
        }
    }

    pub fn folder_workspaces(&self, profile: &str, rows: &[(&str, &Path)]) {
        let rows: Vec<Value> = rows.iter().map(|(id, path)| json!({ "id": id, "name": "f", "folderPath": path })).collect();
        self.set_document(profile, "folderWorkspaces", &Value::Array(rows), 1);
    }

    pub fn chat(&self, row: &ChatRow<'_>) {
        let chain: Vec<Value> = row
            .handles
            .iter()
            .enumerate()
            .map(|(index, handle)| json!({ "linkId": format!("l{index}"), "handle": handle.json(), "origin": "created", "mintedAtFence": 2, "observedAt": 1 }))
            .collect();
        let mut record = json!({
            "schemaVersion": 2,
            "sessionId": row.id,
            "location": { "executionHostId": row.host, "wslDistro": null, "workspaceId": row.workspace, "workspaceKind": "git-worktree" },
            "provider": row.agent,
            "options": { "model": "opencode/big-pickle" },
            "permissionMode": "full-access",
            "createdAt": row.created_ms,
            "updatedAt": row.updated_ms,
            "providerHandleChain": chain,
            "lease": { "claimKeyId": CANARY },
            "acquiredAccount": { "recipe": { "secret": CANARY } },
            "lastTurn": { "state": "settled", "prompt": "p", "reply": "r" },
        });
        if let Some(title) = row.title {
            record["conversationName"] = json!(title);
        }
        if row.renamed {
            record["conversationNameSource"] = json!("user");
        }
        if let Some(launch) = row.launch {
            record["launchDirectory"] = json!(launch);
        }
        if let Some((variable, path)) = row.account_home {
            record["accountHome"] = json!({ "variable": variable, "path": path });
        }
        self.chats
            .execute("INSERT OR REPLACE INTO agent_session_records(session_id, record_json) VALUES(?1, ?2)", params![row.id, record.to_string()])
            .unwrap();
    }

    /// Journal rows for a chat at `epoch`, which becomes its current one.
    pub fn items(&self, session: &str, epoch: &str, payloads: &[Value]) {
        self.chats
            .execute(
                "INSERT OR REPLACE INTO journal_chat_state(session_id, workspace_id, epoch) VALUES(?1, 'w', ?2)",
                params![session, epoch],
            )
            .unwrap();
        self.items_at(session, epoch, payloads, false);
    }

    /// Journal rows at an epoch, without making it current.
    pub fn items_at(&self, session: &str, epoch: &str, payloads: &[Value], hidden: bool) {
        for (index, payload) in payloads.iter().enumerate() {
            let id = payload.get("itemId").and_then(Value::as_str).unwrap().to_owned();
            self.chats
                .execute(
                    "INSERT INTO journal_items(session_id, epoch, item_id, ordinal, revision, sequence, sequence_index, owner_fence, hidden, payload_json)
                     VALUES(?1, ?2, ?3, ?4, 0, ?5, 0, 1, ?6, ?7)",
                    params![session, epoch, id, index as i64, (index as i64 + 1) * 2, i64::from(hidden), payload.to_string()],
                )
                .unwrap();
        }
    }

    pub fn delete_chat(&self, id: &str) {
        let value = json!({ "chatId": id, "sessionIds": [id], "conversations": [] });
        self.chats
            .execute("INSERT OR REPLACE INTO agent_session_store_meta(key, value) VALUES(?1, ?2)", params![format!("chat.deleted:{id}"), value.to_string()])
            .unwrap();
    }

    pub fn run(&self, id: &str, coordinator: Option<&str>) {
        self.runs
            .execute(
                "INSERT INTO runs(id, objective, coordinator_handle, coordinator_orca_session_id, coordinator_orca_session_id_generation, consumer_generation) VALUES(?1, 'Add a LICENSE file', 'term_x', ?2, 1, 1)",
                params![id, coordinator],
            )
            .unwrap();
    }

    pub fn chat_worker(&self, run: &str, dispatch: &str, chat: &str) {
        self.runs
            .execute(
                "INSERT INTO dispatch_contexts(id, run_id, task_id, assignee_orca_session_id, status) VALUES(?1, ?2, 'task_1', ?3, 'completed')",
                params![dispatch, run, chat],
            )
            .unwrap();
    }

    /// A worker in a terminal; `session` is what Orca captured, if anything.
    pub fn terminal_worker(&self, run: &str, dispatch: &str, agent: &str, session: Option<&str>) {
        self.runs
            .execute(
                "INSERT INTO dispatch_contexts(id, run_id, task_id, assignee_handle, status) VALUES(?1, ?2, 'task_1', 'term_w', 'completed')",
                params![dispatch, run],
            )
            .unwrap();
        let resume = session.map(|id| {
            json!({
                "agent": agent,
                "providerSession": { "key": "session_id", "id": id },
                "launchConfig": { "agentArgs": "", "agentEnv": { "ANTHROPIC_API_KEY": CANARY } }
            })
            .to_string()
        });
        self.runs
            .execute(
                "INSERT INTO worker_terminal_resources(id, origin_dispatch_id, owner_dispatch_id, terminal_handle, resume_session) VALUES(?1, ?2, ?2, 'term_w', ?3)",
                params![format!("wtr_{dispatch}"), dispatch, resume],
            )
            .unwrap();
    }
}

/// Journal payloads, as Orca writes them.
pub fn message(id: &str, role: &str, text: &str, at: i64) -> Value {
    json!({ "itemId": id, "revision": 0, "sequence": 1, "observedAt": at, "body": { "kind": "message", "role": role, "blocks": [{ "type": "text", "text": text }] } })
}

pub fn tool(id: &str, name: &str, input: Value, output: &str, state: &str) -> Value {
    json!({ "itemId": id, "revision": 3, "sequence": 1, "observedAt": 1_795_419_170_000_i64, "body": {
        "kind": "tool-call", "callId": format!("call-{id}"), "name": name, "input": input, "state": state,
        "output": { "head": output, "byteLength": output.len(), "digest": "d", "truncated": false } } })
}

pub fn turn(id: &str) -> Value {
    json!({ "itemId": id, "revision": 4, "body": { "kind": "turn", "turnId": "t", "state": "completed", "outcome": "success" } })
}

pub fn status(id: &str, text: &str) -> Value {
    json!({ "itemId": id, "revision": 0, "body": { "kind": "status", "text": text } })
}

/// A subagent's message, which belongs to its own transcript.
pub fn subagent(id: &str, text: &str) -> Value {
    let mut payload = message(id, "assistant", text, 1_795_419_170_000);
    payload["agentId"] = json!("agent-1");
    payload
}
