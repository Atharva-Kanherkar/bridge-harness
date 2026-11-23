CREATE TABLE runs (
  id                    TEXT PRIMARY KEY,
  objective             TEXT NOT NULL,
  home_database         TEXT NOT NULL DEFAULT 'this_database',
  coordinator_handle    TEXT,
  coordinator_pane_key  TEXT,
  -- Bare Orca session id the coordinator is addressed by, when it has one (today only structured
  -- sessions); for a /clear'd chat, its lineage root's.
  coordinator_orca_session_id TEXT,
  -- The consumer_generation coordinator_orca_session_id was written at; the id counts only while they
  -- are equal (run-coordinator-orca-session). So bump consumer_generation for a rebind or unbind only.
  coordinator_orca_session_id_generation INTEGER,
  consumer_generation   INTEGER NOT NULL DEFAULT 0,
  legacy                INTEGER NOT NULL DEFAULT 0,
  created_at            TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at            TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE messages (
  id            TEXT NOT NULL,
  run_id        TEXT NOT NULL DEFAULT 'run_legacy_local',
  delivery_contract TEXT NOT NULL DEFAULT 'current_delivery'
    CHECK(delivery_contract IN ('legacy_direct', 'current_delivery', 'audit_only')),
  from_handle   TEXT NOT NULL,
  to_handle     TEXT NOT NULL,
  subject       TEXT NOT NULL,
  body          TEXT NOT NULL DEFAULT '',
  type          TEXT NOT NULL DEFAULT 'status'
    CHECK(type IN (
      'status', 'dispatch', 'worker_done', 'merge_ready',
      'escalation', 'handoff', 'decision_gate', 'question', 'heartbeat'
    )),
  priority      TEXT NOT NULL DEFAULT 'normal'
    CHECK(priority IN ('normal', 'high', 'urgent')),
  thread_id     TEXT,
  payload       TEXT,
  read          INTEGER NOT NULL DEFAULT 0,
  sequence      INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  delivered_at  TEXT,
  sender_pane_key TEXT,
  pointer_enter_pending INTEGER NOT NULL DEFAULT 0,
  pointer_pty_id TEXT,
  pointer_process_incarnation TEXT
, answer_sender_snapshot TEXT);
CREATE UNIQUE INDEX idx_messages_id ON messages(id);
CREATE INDEX idx_inbox ON messages(to_handle, read);
CREATE INDEX idx_thread ON messages(thread_id);
CREATE TABLE run_coordinator_handles (
  run_id          TEXT NOT NULL,
  terminal_handle TEXT NOT NULL,
  first_bound_at  TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (run_id, terminal_handle)
);
CREATE INDEX idx_run_coordinator_handles_handle
  ON run_coordinator_handles(terminal_handle, run_id);
CREATE TRIGGER trg_runs_forget_coordinator_handles
AFTER DELETE ON runs
BEGIN
  DELETE FROM run_coordinator_handles WHERE run_id = OLD.id;
END;
CREATE TABLE deliveries (
  id                    TEXT PRIMARY KEY,
  run_id                TEXT NOT NULL,
  -- Default keeps a downgraded binary's column-less INSERT working against a v34 database.
  mailbox_handle        TEXT NOT NULL DEFAULT '',
  consumer_generation   INTEGER NOT NULL,
  message_ids           TEXT NOT NULL,
  status                TEXT NOT NULL DEFAULT 'outstanding'
    CHECK(status IN ('outstanding', 'acknowledged', 'fenced')),
  created_at            TEXT NOT NULL DEFAULT (datetime('now')),
  acknowledged_at       TEXT
);
CREATE INDEX idx_deliveries_run_created
  ON deliveries(run_id, created_at);
CREATE TABLE mutation_receipts (
  caller_fingerprint  TEXT NOT NULL,
  request_id          TEXT NOT NULL,
  method              TEXT NOT NULL,
  payload_hash        TEXT NOT NULL,
  state               TEXT NOT NULL DEFAULT 'pending'
    CHECK(state IN ('pending', 'completed')),
  receipt             TEXT,
  created_at          TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (caller_fingerprint, request_id)
);
CREATE TABLE mutation_caller_identities (
  transport           TEXT PRIMARY KEY,
  caller_fingerprint  TEXT NOT NULL UNIQUE
);
CREATE TABLE attempt_observation_facts (
  id                    TEXT PRIMARY KEY,
  dispatch_id           TEXT NOT NULL,
  task_id               TEXT NOT NULL,
  sequence              INTEGER NOT NULL,
  authority_id          TEXT NOT NULL,
  authority_clock       TEXT NOT NULL,
  facet                 TEXT NOT NULL,
  payload               TEXT NOT NULL,
  source_observed_at    INTEGER,
  execution_received_at INTEGER,
  home_received_at      INTEGER NOT NULL,
  created_at            TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(dispatch_id, sequence)
);
CREATE INDEX idx_attempt_observation_facts_projection
  ON attempt_observation_facts(dispatch_id, facet, sequence);
CREATE TABLE worker_dispatches (
  dispatch_id            TEXT PRIMARY KEY,
  runtime_epoch          TEXT,
  state                  TEXT NOT NULL DEFAULT 'starting'
    CHECK(state IN (
      'starting', 'ready', 'start_unknown', 'failed', 'succeeded',
      'stopping', 'stop_unknown', 'stopped', 'abandoned'
    )),
  stage                  TEXT NOT NULL DEFAULT 'accepted',
  worktree_id            TEXT,
  agent_terminal_handle  TEXT,
  setup_state            TEXT NOT NULL DEFAULT 'not_applicable',
  effects                TEXT NOT NULL DEFAULT '[]',
  residual_resources     TEXT NOT NULL DEFAULT '[]',
  start_options          TEXT NOT NULL DEFAULT '{}',
  last_error             TEXT,
  created_at             TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at             TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_worker_dispatches_recoverable
  ON worker_dispatches(dispatch_id)
  WHERE state IN ('starting', 'ready', 'start_unknown', 'stopping', 'stop_unknown');
CREATE TABLE worker_terminal_resources (
  id                       TEXT PRIMARY KEY,
  origin_dispatch_id       TEXT NOT NULL,
  owner_dispatch_id        TEXT NOT NULL,
  prior_owner_dispatch_ids TEXT NOT NULL DEFAULT '[]',
  worktree_id              TEXT,
  terminal_handle          TEXT NOT NULL,
  pane_key                 TEXT,
  process_incarnation      TEXT,
  endpoint_id              TEXT,
  endpoint_incarnation     TEXT,
  host_scope               TEXT,
  ownership_state          TEXT NOT NULL DEFAULT 'owned'
    CHECK(ownership_state IN ('owned', 'transferred', 'user_owned', 'external', 'released')),
  release_state            TEXT NOT NULL DEFAULT 'not_requested'
    CHECK(release_state IN (
      'not_requested', 'retained', 'requested', 'releasing', 'released', 'unknown'
    )),
  retained_reason          TEXT,
  release_requested_at     TEXT,
  release_completed_at     TEXT,
  release_error            TEXT,
  recovery_attempt_count   INTEGER NOT NULL DEFAULT 0,
  last_recovery_at         TEXT,
  archive_source           TEXT,
  archive_status           TEXT,
  auto_release_at          TEXT,
  auto_release_dispatch_id TEXT,
  created_at               TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at               TEXT NOT NULL DEFAULT (datetime('now')),
  -- JSON: the agent conversation a closed worker reopens in (WorkerResumeSession).
  resume_session           TEXT
);
CREATE UNIQUE INDEX idx_worker_terminal_resources_owner
  ON worker_terminal_resources(owner_dispatch_id);
CREATE INDEX idx_worker_terminal_resources_handle
  ON worker_terminal_resources(terminal_handle);
CREATE INDEX idx_worker_terminal_resources_pane
  ON worker_terminal_resources(pane_key);
CREATE INDEX idx_worker_terminal_resources_identity
  ON worker_terminal_resources(process_incarnation, host_scope);
CREATE INDEX idx_worker_terminal_resources_release
  ON worker_terminal_resources(release_state);
CREATE TABLE structured_pointer_operations (
  mailbox_handle    TEXT PRIMARY KEY,
  session_id        TEXT NOT NULL,
  operation_id      TEXT NOT NULL,
  batch_fingerprint  TEXT NOT NULL,
  minted_at_ms      INTEGER NOT NULL
);
CREATE TABLE structured_mail_sends (
  mailbox_handle TEXT NOT NULL,
  message_id     TEXT NOT NULL,
  session_id     TEXT NOT NULL,
  operation_id   TEXT NOT NULL,
  minted_at_ms   INTEGER NOT NULL,
  PRIMARY KEY (mailbox_handle, message_id)
);
CREATE INDEX idx_structured_mail_sends_session ON structured_mail_sends(session_id);
CREATE INDEX idx_structured_mail_sends_message ON structured_mail_sends(message_id);
CREATE TABLE orchestration_maintenance (
  id        INTEGER PRIMARY KEY CHECK(id = 1),
  pruned_at INTEGER
);
CREATE TABLE federated_dispatches (
  dispatch_id             TEXT PRIMARY KEY,
  environment_id          TEXT NOT NULL,
  environment_name        TEXT NOT NULL,
  peer_fingerprint        TEXT NOT NULL,
  remote_runtime_epoch    TEXT,
  protocol_version        INTEGER NOT NULL DEFAULT 1,
  remote_worktree_id      TEXT,
  remote_terminal_handle  TEXT,
  to_home_imported_sequence INTEGER NOT NULL DEFAULT 0,
  to_home_acknowledged_sequence INTEGER NOT NULL DEFAULT 0,
  home_auto_release_at    TEXT,
  home_auto_release_state TEXT,
  created_at              TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at              TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE remote_dispatch_attachments (
  -- DEFAULT: a rolled-back v1.4.198 host still inserts here without a home Run.
  home_run_id             TEXT NOT NULL DEFAULT '',
  dispatch_id             TEXT PRIMARY KEY,
  task_id                 TEXT NOT NULL,
  home_peer_fingerprint   TEXT NOT NULL,
  protocol_version        INTEGER NOT NULL DEFAULT 1,
  runtime_epoch           TEXT NOT NULL,
  capability_hash         TEXT,
  pane_key                TEXT,
  process_incarnation     TEXT,
  state                   TEXT NOT NULL DEFAULT 'starting'
    CHECK(state IN (
      'starting', 'ready', 'start_unknown', 'failed', 'succeeded',
      'stopping', 'stop_unknown', 'stopped', 'abandoned'
    )),
  stage                   TEXT NOT NULL DEFAULT 'accepted',
  worktree_id             TEXT,
  terminal_handle         TEXT,
  setup_state             TEXT NOT NULL DEFAULT 'not_applicable',
  effects                 TEXT NOT NULL DEFAULT '[]',
  residual_resources      TEXT NOT NULL DEFAULT '[]',
  to_worker_imported_sequence INTEGER NOT NULL DEFAULT 0,
  -- Nesting depth of the worker this attachment represents. Propagated from the
  -- Run home; absent from an old client means 1, which fails closed.
  depth                   INTEGER NOT NULL DEFAULT 1,
  -- Its own counter: a federated worker host has no dispatch_contexts row to borrow one from.
  consumer_generation     INTEGER NOT NULL DEFAULT 0,
  last_error              TEXT,
  created_at              TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at              TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE federation_relay_items (
  dispatch_id   TEXT NOT NULL,
  direction     TEXT NOT NULL CHECK(direction IN ('to_home', 'to_worker')),
  sequence      INTEGER NOT NULL,
  message_id    TEXT NOT NULL,
  kind          TEXT NOT NULL,
  payload       TEXT NOT NULL,
  byte_count    INTEGER NOT NULL,
  acked_at      TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')), answer_sender_report TEXT,
  PRIMARY KEY (dispatch_id, direction, sequence),
  UNIQUE (dispatch_id, direction, message_id)
);
CREATE INDEX idx_federation_relay_pending
  ON federation_relay_items(dispatch_id, direction, acked_at, sequence);
CREATE INDEX idx_federation_relay_unacked
  ON federation_relay_items(direction, dispatch_id) WHERE acked_at IS NULL;
CREATE TABLE remote_questions (
  message_id        TEXT PRIMARY KEY,
  dispatch_id       TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'answered', 'closed')),
  answer_message_id TEXT,
  answer_body       TEXT,
  created_at        TEXT NOT NULL DEFAULT (datetime('now')),
  answered_at       TEXT
);
CREATE INDEX idx_remote_questions_dispatch_status
  ON remote_questions(dispatch_id, status);
CREATE TABLE peer_mail_conversations (
  conversation_id  TEXT PRIMARY KEY,
  role             TEXT NOT NULL CHECK(role IN ('home', 'peer')),
  environment_id   TEXT,
  peer_fingerprint TEXT NOT NULL,
  local_party      TEXT,
  local_pane_key   TEXT,
  created_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX idx_peer_mail_home_pairing
  ON peer_mail_conversations(environment_id, local_party, peer_fingerprint) WHERE role = 'home';
CREATE INDEX idx_peer_mail_peer_fingerprint
  ON peer_mail_conversations(peer_fingerprint, role);
CREATE TABLE peer_mail_messages (
  message_id      TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  sender_label    TEXT NOT NULL,
  remote_party    TEXT
);
CREATE INDEX idx_peer_mail_messages_sender ON peer_mail_messages(sender_label);
CREATE TABLE tasks (
  id            TEXT PRIMARY KEY,
  run_id        TEXT NOT NULL DEFAULT 'run_legacy_local',
  parent_id     TEXT,
  created_by_terminal_handle TEXT,
  created_by_pane_key TEXT,
  created_by_process_incarnation TEXT,
  created_by_run_generation INTEGER,
  task_title    TEXT,
  display_name  TEXT,
  spec          TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN (
      'pending', 'ready', 'dispatched',
      'completed', 'failed', 'blocked'
    )),
  deps          TEXT NOT NULL DEFAULT '[]',
  result        TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at  TEXT
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent_id);
CREATE TABLE dispatch_contexts (
  id                  TEXT PRIMARY KEY,
  run_id              TEXT NOT NULL DEFAULT 'run_legacy_local',
  task_id             TEXT NOT NULL,
  contract_version    INTEGER NOT NULL DEFAULT 1,
  launch_token_hash   TEXT,
  assignee_handle     TEXT,
  assignee_pane_key   TEXT,
  -- Bare Orca session id the agent is addressed by, when it has one (today only structured
  -- sessions); for a /clear'd chat, its lineage root's. Not its orca_session_id:<id> address.
  assignee_orca_session_id TEXT,
  capability_hash     TEXT,
  process_incarnation TEXT,
  capability_revoked_at TEXT,
  -- R1 identity facts; nullable when legacy provenance was never proven.
  retry_of_dispatch_id TEXT,
  creator_dispatch_id TEXT,
  -- Who created this row. A row whose creator is its own assignee is bookkeeping, not delegation,
  -- so it must not count as a nesting parent. Null on rows written before v37 and for Orca's loop.
  creator_handle      TEXT,
  creator_pane_key    TEXT,
  -- Same form as assignee_orca_session_id: the id the creator is addressed by (a lineage root's).
  creator_orca_session_id TEXT,
  -- 'connector' when the creator chat's turn was its connector conversation's, so reports go back there.
  creator_turn_source TEXT,
  -- What its creator started it for, opaque here (bot items: bot-item:<uid>); retries inherit it.
  origin_ref          TEXT,
  host_scope          TEXT,
  status              TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'dispatched', 'completed', 'failed', 'circuit_broken')),
  failure_count       INTEGER NOT NULL DEFAULT 0,
  last_failure        TEXT,
  -- Why the process is gone, when Orca could establish it. See TerminalExitCause.
  termination_reason  TEXT,
  -- Nesting depth: a root coordinator's worker is 1, its worker's worker is 2.
  -- Defaults to 1 so an unstamped row fails closed rather than reading as a root.
  depth               INTEGER NOT NULL DEFAULT 1,
  -- Bumped whenever the Dispatch is re-pointed at a pane/process, fencing the prior consumer's
  -- outstanding dispatch mailbox Delivery.
  consumer_generation INTEGER NOT NULL DEFAULT 0,
  dispatched_at       TEXT,
  completed_at        TEXT,
  created_at          TEXT NOT NULL DEFAULT (datetime('now')),
  last_heartbeat_at   TEXT,
  -- The settled Dispatch whose worker this one continues (a follow-up sent to it).
  follow_up_of_dispatch_id TEXT,
  -- Set when the worker reported with coordinator mail unread; cleared once that mail continues it.
  follow_up_mail_at TEXT
, for_message_id TEXT);
CREATE INDEX idx_dispatch_task ON dispatch_contexts(task_id);
CREATE INDEX idx_dispatch_status ON dispatch_contexts(status);
CREATE INDEX idx_dispatch_assignee_handle ON dispatch_contexts(assignee_handle);
CREATE TABLE decision_gates (
  id            TEXT PRIMARY KEY,
  run_id        TEXT NOT NULL DEFAULT 'run_legacy_local',
  task_id       TEXT NOT NULL,
  question      TEXT NOT NULL,
  options       TEXT NOT NULL DEFAULT '[]',
  status        TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'resolved', 'timeout')),
  resolution    TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  resolved_at   TEXT
);
CREATE INDEX idx_gates_task ON decision_gates(task_id);
CREATE INDEX idx_gates_status ON decision_gates(status);
CREATE INDEX idx_runs_coordinator_pane_leaf
  ON runs(substr(coordinator_pane_key, instr(coordinator_pane_key, ':') + 1))
  WHERE coordinator_pane_key IS NOT NULL;
CREATE TABLE coordinator_runs (
  id                  TEXT PRIMARY KEY,
  spec                TEXT NOT NULL,
  status              TEXT NOT NULL DEFAULT 'idle'
    CHECK(status IN ('idle', 'running', 'completed', 'failed')),
  coordinator_handle  TEXT NOT NULL,
  poll_interval_ms    INTEGER NOT NULL DEFAULT 2000,
  created_at          TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at        TEXT,
  scheduler_lost_at   TEXT
);
CREATE TABLE removed_orchestration_work (
  seq        INTEGER PRIMARY KEY,
  run_id     TEXT,
  kind       TEXT NOT NULL,
  entity_id  TEXT NOT NULL,
  record     TEXT NOT NULL,
  removed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_removed_orchestration_work_run
  ON removed_orchestration_work(run_id, seq);
CREATE INDEX idx_removed_orchestration_work_entity
  ON removed_orchestration_work(entity_id, kind);
CREATE TABLE removed_orchestration_work_coverage (
  singleton      INTEGER PRIMARY KEY CHECK(singleton = 1),
  recorded_since TEXT NOT NULL
);
CREATE TRIGGER trg_runs_forget_removed_work
AFTER DELETE ON runs
BEGIN
  DELETE FROM removed_orchestration_work WHERE run_id = OLD.id;
END;
CREATE TABLE bot_requests (
  message_id  TEXT PRIMARY KEY,
  root_key    TEXT NOT NULL,
  from_bot_id TEXT,
  to_bot_id   TEXT NOT NULL,
  purpose     TEXT NOT NULL CHECK(purpose IN ('status', 'handoff', 'question')),
  depth       INTEGER NOT NULL,
  record      TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  send_key    TEXT
, taken_by TEXT, awaiting INTEGER);
CREATE TABLE bot_request_outcomes (
  message_id  TEXT PRIMARY KEY,
  outcome     TEXT NOT NULL CHECK(outcome IN ('answered', 'posted', 'unanswered', 'dropped')),
  recorded_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE bot_conversation_acts_for (
  session_id   TEXT PRIMARY KEY,
  acts_for     TEXT NOT NULL,
  triggered_by TEXT,
  created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE bot_run_resends (
  run_id     TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE remote_run_links (
  conversation_id          TEXT PRIMARY KEY,
  run_id                   TEXT NOT NULL,
  environment_id           TEXT NOT NULL,
  peer_fingerprint         TEXT NOT NULL,
  home_fingerprint         TEXT NOT NULL,
  pairing_revision         INTEGER,
  peer_run_id              TEXT,
  -- The server's name when the link was made, for a server removed from this Orca since.
  server_name              TEXT,
  state                    TEXT NOT NULL DEFAULT 'open'
    CHECK(state IN ('open', 'cancel_owed', 'closed')),
  cancel_key               TEXT,
  cancel_attempt           INTEGER NOT NULL DEFAULT 0,
  cancel_retries           INTEGER NOT NULL DEFAULT 0,
  cancel_retry_at          INTEGER,
  last_imported_sequence   INTEGER NOT NULL DEFAULT 0,
  last_error               TEXT,
  created_at               TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX idx_remote_run_links_open
  ON remote_run_links(run_id, environment_id) WHERE state = 'open';
CREATE INDEX idx_remote_run_links_environment
  ON remote_run_links(environment_id, state);
CREATE TABLE remote_placements (
  task_id            TEXT PRIMARY KEY,
  run_id             TEXT NOT NULL,
  environment_id     TEXT NOT NULL,
  conversation_id    TEXT NOT NULL,
  state              TEXT NOT NULL CHECK(state IN ('reserved', 'accepted')),
  request_key        TEXT NOT NULL,
  prior_task_status  TEXT NOT NULL,
  dispatch_id        TEXT,
  created_at         TEXT NOT NULL DEFAULT (datetime('now')),
  -- Set when every send under request_key failed before leaving this Orca, so a cancel may retire
  -- the reservation; a resend clears it first.
  never_sent_at      TEXT
);
CREATE INDEX idx_remote_placements_conversation ON remote_placements(conversation_id);
CREATE TABLE remote_dispatches (
  dispatch_id        TEXT PRIMARY KEY,
  conversation_id    TEXT NOT NULL,
  run_id             TEXT NOT NULL,
  environment_id     TEXT NOT NULL,
  task_id            TEXT NOT NULL,
  status             TEXT NOT NULL DEFAULT 'pending',
  worker_state       TEXT,
  last_failure       TEXT,
  agent              TEXT,
  last_report        TEXT,
  stop_owed          INTEGER NOT NULL DEFAULT 0,
  stop_count         INTEGER NOT NULL DEFAULT 0,
  stop_attempt       INTEGER NOT NULL DEFAULT 0,
  stop_retries       INTEGER NOT NULL DEFAULT 0,
  stop_retry_at      INTEGER,
  stop_last_error    TEXT,
  -- This Orca already ran the Task half for the attempt: 1 for a stop the server did not answer,
  -- 2 once it was abandoned here. The first result stands: later states from the server change
  -- only this record. Abandoned here, the attempt is settled for a retry, as a local abandon is.
  task_settled_here  INTEGER NOT NULL DEFAULT 0,
  created_at         TEXT NOT NULL DEFAULT (datetime('now')),
  -- When this Orca recorded the server's start answer or, after that, its latest state for the
  -- Dispatch. Reports and this Orca's own stop bookkeeping leave it.
  updated_at         TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_remote_dispatches_run ON remote_dispatches(run_id);
CREATE INDEX idx_remote_dispatches_task ON remote_dispatches(task_id);
CREATE INDEX idx_remote_dispatches_conversation ON remote_dispatches(conversation_id);
CREATE INDEX idx_remote_dispatches_stop_owed
  ON remote_dispatches(environment_id) WHERE stop_owed = 1;
CREATE TABLE remote_stop_intents (
  dispatch_id  TEXT NOT NULL,
  intent_key   TEXT NOT NULL,
  stop_count   INTEGER NOT NULL,
  PRIMARY KEY (dispatch_id, intent_key)
);
CREATE TABLE remote_owned_questions (
  message_id       TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  run_id           TEXT NOT NULL,
  environment_id   TEXT NOT NULL,
  dispatch_id      TEXT NOT NULL
, taken_by TEXT, awaiting INTEGER, ended TEXT CHECK(ended IN ('answered', 'unanswered', 'dropped')));
CREATE INDEX idx_remote_owned_questions_run ON remote_owned_questions(run_id);
CREATE TABLE peer_runs (
  run_id           TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL UNIQUE,
  peer_fingerprint TEXT NOT NULL,
  home_run_id      TEXT NOT NULL,
  home_label       TEXT NOT NULL,
  state            TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open', 'closed')),
  created_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE federation_relay_sequences (
  dispatch_id   TEXT NOT NULL,
  direction     TEXT NOT NULL,
  last_sequence INTEGER NOT NULL,
  PRIMARY KEY (dispatch_id, direction)
);
CREATE TABLE peer_run_corrections (
  dispatch_id TEXT PRIMARY KEY
);
CREATE TABLE message_dispatch_provenance (
  message_id      TEXT PRIMARY KEY,
  dispatch_id     TEXT NOT NULL,
  task_id         TEXT NOT NULL,
  sender_snapshot TEXT
);
CREATE TABLE worker_presence (
  dispatch_id          TEXT PRIMARY KEY,
  presence             TEXT NOT NULL CHECK(presence IN ('working', 'resting', 'prompt', 'gone')),
  generation           INTEGER NOT NULL,
  fence                TEXT,
  report               TEXT,
  reported_generation  INTEGER,
  reported_mail_count  INTEGER,
  updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX worker_presence_undecided ON worker_presence(dispatch_id)
  WHERE report IS NOT NULL AND presence != 'working';
CREATE INDEX idx_dispatch_session_presentation
      ON dispatch_contexts(status, completed_at) WHERE assignee_orca_session_id IS NOT NULL;
CREATE INDEX idx_messages_crew_from
        ON messages(from_handle, julianday(created_at), to_handle, sequence) WHERE type <> 'heartbeat';
CREATE INDEX idx_messages_crew_to
        ON messages(to_handle, julianday(created_at), from_handle, sequence) WHERE type <> 'heartbeat';
CREATE INDEX idx_dispatch_crew_recent
      ON dispatch_contexts(julianday(completed_at)) WHERE status NOT IN ('pending', 'dispatched');
CREATE INDEX idx_crew_dispatch_pane
      ON dispatch_contexts(assignee_pane_key) WHERE assignee_pane_key IS NOT NULL;
CREATE INDEX idx_crew_dispatch_leaf
      ON dispatch_contexts(substr(assignee_pane_key, instr(assignee_pane_key, ':') + 1)) WHERE assignee_pane_key IS NOT NULL;
CREATE INDEX idx_crew_remote_handle ON remote_dispatch_attachments(terminal_handle);
CREATE INDEX idx_crew_remote_pane ON remote_dispatch_attachments(pane_key);
CREATE INDEX idx_crew_remote_leaf ON remote_dispatch_attachments(substr(pane_key, instr(pane_key, ':') + 1)) WHERE pane_key IS NOT NULL;
CREATE INDEX idx_crew_resource_leaf
      ON worker_terminal_resources(substr(pane_key, instr(pane_key, ':') + 1)) WHERE pane_key IS NOT NULL;
CREATE INDEX idx_messages_undelivered_inbox
        ON messages(to_handle, read, delivered_at, sequence)
    ;
CREATE INDEX idx_messages_pending_pointer_enter
        ON messages(to_handle, sequence)
        WHERE read = 0 AND pointer_enter_pending > 0;
CREATE INDEX idx_messages_pending_pointer_pty
          ON messages(pointer_pty_id) WHERE pointer_enter_pending > 0;
CREATE INDEX idx_messages_undelivered_direct_run
      ON messages(run_id, to_handle, sequence)
      WHERE read = 0 AND delivered_at IS NULL
        AND delivery_contract = 'current_delivery';
CREATE INDEX idx_messages_unread_current_inbox
      ON messages(to_handle, sequence)
      WHERE read = 0 AND delivery_contract = 'current_delivery';
CREATE INDEX idx_messages_unread_current_inbox_type
      ON messages(to_handle, type, sequence)
      WHERE read = 0 AND delivery_contract = 'current_delivery';
CREATE INDEX idx_messages_unread_current_run_type
      ON messages(run_id, to_handle, type, sequence)
      WHERE read = 0 AND delivery_contract = 'current_delivery';
CREATE INDEX idx_messages_run_sequence ON messages(run_id, sequence);
CREATE INDEX idx_tasks_run_status ON tasks(run_id, status);
CREATE INDEX idx_dispatch_run_status ON dispatch_contexts(run_id, status);
CREATE INDEX idx_gates_run_status ON decision_gates(run_id, status);
CREATE INDEX idx_runs_coordinator_pane ON runs(coordinator_pane_key);
CREATE TABLE question_threads (
      message_id                TEXT PRIMARY KEY,
      run_id                    TEXT NOT NULL,
      dispatch_id               TEXT NOT NULL,
      asker_handle              TEXT NOT NULL,
      status                    TEXT NOT NULL DEFAULT 'pending'
        CHECK(status IN ('pending', 'answered', 'closed')),
      answer_message_id         TEXT,
      answer_body               TEXT,
      answered_by_generation    INTEGER,
      created_at                TEXT NOT NULL DEFAULT (datetime('now')),
      answered_at               TEXT,
      closed_at                 TEXT
    , taken_by TEXT, awaiting INTEGER, told TEXT CHECK(told IN ('unanswered', 'dropped')));
CREATE INDEX idx_questions_dispatch_status
      ON question_threads(dispatch_id, status);
CREATE INDEX idx_messages_delivery_contract
      ON messages(run_id, delivery_contract, to_handle, read, sequence);
CREATE TABLE legacy_adoptions (
      source_run_id        TEXT PRIMARY KEY,
      adopted_run_id       TEXT UNIQUE NOT NULL,
      scheduler_state_lost INTEGER NOT NULL,
      adopted_at           TEXT NOT NULL DEFAULT (datetime('now'))
    );
CREATE TABLE legacy_compatibility_principals (
      id                  TEXT PRIMARY KEY,
      run_id              TEXT NOT NULL,
      dispatch_id         TEXT,
      role                TEXT NOT NULL CHECK(role IN ('worker', 'coordinator')),
      host_scope          TEXT NOT NULL,
      terminal_handle     TEXT NOT NULL,
      pane_key            TEXT NOT NULL,
      launch_token_hash   TEXT NOT NULL,
      process_incarnation TEXT,
      status              TEXT NOT NULL
        CHECK(status IN ('committed', 'settled', 'revoked')),
      CHECK(
        (role = 'worker' AND dispatch_id IS NOT NULL) OR
        (role = 'coordinator' AND dispatch_id IS NULL)
      ),
      UNIQUE(role, run_id, dispatch_id)
    );
CREATE UNIQUE INDEX idx_legacy_principal_coordinator
      ON legacy_compatibility_principals(run_id)
      WHERE role = 'coordinator';
CREATE UNIQUE INDEX idx_legacy_principal_dispatch
      ON legacy_compatibility_principals(dispatch_id)
      WHERE role = 'worker';
CREATE TABLE legacy_operation_receipts (
      principal_id   TEXT NOT NULL,
      operation_key  TEXT NOT NULL,
      method         TEXT NOT NULL,
      payload_hash   TEXT NOT NULL,
      effect_id      TEXT NOT NULL,
      response_json  TEXT NOT NULL,
      completed_at   TEXT NOT NULL DEFAULT (datetime('now')),
      PRIMARY KEY(principal_id, operation_key)
    );
CREATE TABLE legacy_mail_receipts (
      principal_id    TEXT NOT NULL,
      message_id      TEXT NOT NULL,
      acknowledged_at TEXT,
      PRIMARY KEY(principal_id, message_id)
    );
CREATE INDEX idx_dispatch_active_assignee_handle
          ON dispatch_contexts(assignee_handle)
          WHERE assignee_handle IS NOT NULL AND status IN ('pending', 'dispatched');
CREATE INDEX idx_mutation_receipts_completed_updated
      ON mutation_receipts(updated_at) WHERE state = 'completed';
CREATE TABLE mutation_receipt_ledger (
      singleton     INTEGER PRIMARY KEY CHECK(singleton = 1),
      receipt_count INTEGER NOT NULL CHECK(receipt_count >= 0)
    );
CREATE TRIGGER mutation_receipts_count_insert
    AFTER INSERT ON mutation_receipts
    BEGIN
      UPDATE mutation_receipt_ledger
      SET receipt_count = receipt_count + 1
      WHERE singleton = 1;
    END;
CREATE TRIGGER mutation_receipts_count_delete
    AFTER DELETE ON mutation_receipts
    BEGIN
      UPDATE mutation_receipt_ledger
      SET receipt_count = receipt_count - 1
      WHERE singleton = 1;
    END;
CREATE INDEX idx_remote_dispatch_attachments_active_pane
          ON remote_dispatch_attachments(pane_key)
          WHERE state IN ('starting', 'ready', 'start_unknown', 'stopping', 'stop_unknown');
CREATE INDEX idx_remote_dispatch_attachments_active_pane_suffix
          ON remote_dispatch_attachments(substr(pane_key, instr(pane_key, ':') + 1))
          WHERE state IN ('starting', 'ready', 'start_unknown', 'stopping', 'stop_unknown')
            AND pane_key IS NOT NULL;
CREATE INDEX idx_dispatch_assignee_pane_leaf
        ON dispatch_contexts(substr(assignee_pane_key, instr(assignee_pane_key, ':') + 1))
        WHERE assignee_pane_key IS NOT NULL AND status IN ('pending', 'dispatched');
CREATE INDEX idx_dispatch_active_run_assignee_handle
        ON dispatch_contexts(run_id, assignee_handle)
        WHERE assignee_handle IS NOT NULL AND status IN ('pending', 'dispatched');
CREATE INDEX idx_dispatch_active_run_pane_leaf
        ON dispatch_contexts(run_id, substr(assignee_pane_key, instr(assignee_pane_key, ':') + 1))
        WHERE assignee_pane_key IS NOT NULL AND status IN ('pending', 'dispatched');
CREATE INDEX idx_dispatch_active_assignee_pane_key
        ON dispatch_contexts(assignee_pane_key)
        WHERE assignee_pane_key IS NOT NULL AND status IN ('pending', 'dispatched');
CREATE INDEX idx_crew_pending_questions
      ON question_threads(message_id) WHERE status = 'pending';
CREATE TRIGGER trg_tasks_delete_additive_lifecycle
AFTER DELETE ON tasks
BEGIN
  DELETE FROM attempt_observation_facts WHERE task_id = OLD.id;
END;
CREATE TRIGGER trg_dispatches_delete_additive_lifecycle
AFTER DELETE ON dispatch_contexts
BEGIN
  DELETE FROM attempt_observation_facts WHERE dispatch_id = OLD.id;
END;
CREATE TABLE IF NOT EXISTS "worker_terminal_archives" (
        dispatch_id   TEXT PRIMARY KEY,
        resource_id   TEXT NOT NULL,
        kind          TEXT NOT NULL CHECK(kind IN ('transcript_pin', 'terminal_tail', 'structured_journal')),
        content       TEXT NOT NULL,
        created_at    TEXT NOT NULL DEFAULT (datetime('now'))
      );
CREATE INDEX idx_deliveries_one_outstanding
    ON deliveries(mailbox_handle) WHERE status = 'outstanding' AND mailbox_handle != '';
CREATE INDEX idx_runs_coordinator_orca_session_id
      ON runs(coordinator_orca_session_id) WHERE coordinator_orca_session_id IS NOT NULL;
CREATE INDEX idx_dispatch_assignee_orca_session_id
      ON dispatch_contexts(assignee_orca_session_id) WHERE assignee_orca_session_id IS NOT NULL;
CREATE INDEX idx_worker_terminal_resources_auto_release
      ON worker_terminal_resources(auto_release_at) WHERE auto_release_at IS NOT NULL;
CREATE INDEX idx_federated_dispatches_home_auto_release
      ON federated_dispatches(home_auto_release_at) WHERE home_auto_release_at IS NOT NULL;
CREATE UNIQUE INDEX idx_dispatch_contexts_follow_up_of
      ON dispatch_contexts(follow_up_of_dispatch_id) WHERE follow_up_of_dispatch_id IS NOT NULL;
CREATE INDEX idx_dispatch_contexts_origin
      ON dispatch_contexts(origin_ref) WHERE origin_ref IS NOT NULL;
CREATE INDEX idx_dispatch_creator_orca_session_id
      ON dispatch_contexts(creator_orca_session_id) WHERE creator_orca_session_id IS NOT NULL;
CREATE INDEX idx_dispatch_creator_pane_key
      ON dispatch_contexts(creator_pane_key) WHERE creator_pane_key IS NOT NULL;
CREATE INDEX idx_bot_requests_send ON bot_requests(send_key);
CREATE INDEX idx_coordinator_runs_scheduler_lost
      ON coordinator_runs(scheduler_lost_at) WHERE scheduler_lost_at IS NOT NULL;
CREATE INDEX idx_dispatch_assignee_run_contract
      ON dispatch_contexts(assignee_handle, run_id, contract_version) WHERE assignee_handle IS NOT NULL;
CREATE TABLE peer_questions_out (
      message_id        TEXT PRIMARY KEY,
      conversation_id   TEXT NOT NULL,
      environment_id    TEXT NOT NULL,
      peer_fingerprint  TEXT NOT NULL,
      asker             TEXT NOT NULL,
      thread_id         TEXT,
      awaiting          INTEGER,
      created_at        TEXT NOT NULL DEFAULT (datetime('now'))
    );
CREATE INDEX idx_bot_requests_awaiting
      ON bot_requests(taken_by) WHERE awaiting = 1;
CREATE INDEX idx_question_threads_awaiting
      ON question_threads(taken_by) WHERE awaiting = 1;
CREATE INDEX idx_dispatch_contexts_for_message
      ON dispatch_contexts(for_message_id) WHERE for_message_id IS NOT NULL;
CREATE INDEX idx_peer_questions_out_awaiting
      ON peer_questions_out(peer_fingerprint) WHERE awaiting = 1;
CREATE INDEX idx_remote_owned_questions_awaiting
      ON remote_owned_questions(taken_by) WHERE awaiting = 1;
CREATE VIEW outstanding_deliveries AS
    SELECT * FROM deliveries
    WHERE status = 'outstanding'
      AND EXISTS (
        SELECT 1 FROM json_each(deliveries.message_ids) AS member
        JOIN messages ON messages.id = member.value WHERE messages.read = 0
      )
/* outstanding_deliveries(id,run_id,mailbox_handle,consumer_generation,message_ids,status,created_at,acknowledged_at) */;
CREATE TRIGGER trg_deliveries_one_outstanding
    AFTER INSERT ON deliveries
    WHEN NEW.mailbox_handle != '' AND EXISTS (
      SELECT 1 FROM outstanding_deliveries
       WHERE mailbox_handle = NEW.mailbox_handle AND mailbox_handle != '' LIMIT 1 OFFSET 1
    )
    BEGIN
      SELECT RAISE(ABORT, 'Mailbox already has an outstanding delivery');
    END;
CREATE TABLE worker_terminal_resource_prior_owners (
  dispatch_id TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  PRIMARY KEY (dispatch_id, resource_id)
) WITHOUT ROWID;
CREATE INDEX idx_worker_terminal_resource_prior_owners_resource
  ON worker_terminal_resource_prior_owners(resource_id);
CREATE TRIGGER trg_worker_terminal_prior_owners_insert
AFTER INSERT ON worker_terminal_resources
BEGIN
  INSERT OR IGNORE INTO worker_terminal_resource_prior_owners (dispatch_id, resource_id)
    SELECT prior.value, NEW.id FROM json_each(CASE WHEN json_valid(NEW.prior_owner_dispatch_ids)
     THEN NEW.prior_owner_dispatch_ids ELSE '[]' END) AS prior WHERE prior.type = 'text';
END;
CREATE TRIGGER trg_worker_terminal_prior_owners_update
AFTER UPDATE OF id, prior_owner_dispatch_ids ON worker_terminal_resources
WHEN OLD.id IS NOT NEW.id OR OLD.prior_owner_dispatch_ids IS NOT NEW.prior_owner_dispatch_ids
BEGIN
  DELETE FROM worker_terminal_resource_prior_owners WHERE resource_id = OLD.id
    AND (OLD.id IS NOT NEW.id OR dispatch_id NOT IN (
      SELECT prior.value FROM json_each(CASE WHEN json_valid(NEW.prior_owner_dispatch_ids)
     THEN NEW.prior_owner_dispatch_ids ELSE '[]' END) AS prior WHERE prior.type = 'text'));
  INSERT OR IGNORE INTO worker_terminal_resource_prior_owners (dispatch_id, resource_id)
    SELECT prior.value, NEW.id FROM json_each(CASE WHEN json_valid(NEW.prior_owner_dispatch_ids)
     THEN NEW.prior_owner_dispatch_ids ELSE '[]' END) AS prior WHERE prior.type = 'text';
END;
CREATE TRIGGER trg_worker_terminal_prior_owners_delete
AFTER DELETE ON worker_terminal_resources
BEGIN
  DELETE FROM worker_terminal_resource_prior_owners WHERE resource_id = OLD.id;
END;
CREATE TRIGGER trg_runs_remember_coordinator_insert
      AFTER INSERT ON runs
      WHEN NEW.legacy = 0
      BEGIN
        INSERT OR IGNORE INTO run_coordinator_handles (run_id, terminal_handle)
        SELECT NEW.id, NEW.coordinator_handle WHERE NEW.coordinator_handle IS NOT NULL;
        INSERT OR IGNORE INTO run_coordinator_handles (run_id, terminal_handle)
        SELECT NEW.id, ('orca_session_id:' || (CASE WHEN NEW.coordinator_orca_session_id_generation = NEW.consumer_generation
    THEN NEW.coordinator_orca_session_id END))
        WHERE ('orca_session_id:' || (CASE WHEN NEW.coordinator_orca_session_id_generation = NEW.consumer_generation
    THEN NEW.coordinator_orca_session_id END)) IS NOT NULL;
      END;
CREATE TRIGGER trg_runs_remember_coordinator_update
      AFTER UPDATE OF coordinator_handle, coordinator_orca_session_id,
        coordinator_orca_session_id_generation ON runs
      WHEN NEW.legacy = 0
      BEGIN
        INSERT OR IGNORE INTO run_coordinator_handles (run_id, terminal_handle)
        SELECT NEW.id, NEW.coordinator_handle WHERE NEW.coordinator_handle IS NOT NULL;
        INSERT OR IGNORE INTO run_coordinator_handles (run_id, terminal_handle)
        SELECT NEW.id, ('orca_session_id:' || (CASE WHEN NEW.coordinator_orca_session_id_generation = NEW.consumer_generation
    THEN NEW.coordinator_orca_session_id END))
        WHERE ('orca_session_id:' || (CASE WHEN NEW.coordinator_orca_session_id_generation = NEW.consumer_generation
    THEN NEW.coordinator_orca_session_id END)) IS NOT NULL;
      END;
CREATE VIEW peer_run_dispatch_state AS
SELECT d.id AS dispatch_id, pr.conversation_id, pr.home_run_id, pr.run_id AS peer_run_id,
  json_object(
    'id', d.id, 'taskId', d.task_id, 'status', d.status, 'workerState', w.state,
    'lastFailure', d.last_failure, 'agent', json_extract(w.start_options, '$.agent')
  ) AS dispatch_json,
  json_object('status', t.status, 'result', t.result) AS task_json
FROM dispatch_contexts d
JOIN peer_runs pr ON pr.run_id = d.run_id
JOIN tasks t ON t.id = d.task_id
LEFT JOIN worker_dispatches w ON w.dispatch_id = d.id
/* peer_run_dispatch_state(dispatch_id,conversation_id,home_run_id,peer_run_id,dispatch_json,task_json) */;
CREATE TRIGGER trg_peer_run_mail_insert AFTER INSERT ON messages
      WHEN NEW.to_handle LIKE 'run:%' AND NEW.type <> 'heartbeat'
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT pr.conversation_id, NEW.id AS message_id,
       json_object(
         'v', 1, 'id', NEW.id,
         'homeRunId', pr.home_run_id, 'peerRunId', pr.run_id,
         'message', json_object(
           'from', NEW.from_handle, 'type', NEW.type, 'subject', NEW.subject,
           'body', NEW.body, 'payload', NEW.payload, 'priority', NEW.priority,
           'threadId', NEW.thread_id
         ),
         'dispatchId', p.dispatch_id, 'taskId', p.task_id
       ) AS payload
     FROM peer_runs pr
     LEFT JOIN message_dispatch_provenance p ON p.message_id = NEW.id
       AND p.dispatch_id IN (SELECT id FROM dispatch_contexts WHERE run_id = pr.run_id)
     WHERE NEW.to_handle = 'run:' || pr.run_id
       AND NOT EXISTS (
         SELECT 1 FROM federation_relay_items
         WHERE dispatch_id = pr.conversation_id AND direction = 'to_home'
           AND message_id = NEW.id
       )) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_mail', payload, length(payload)
     FROM (SELECT pr.conversation_id, NEW.id AS message_id,
       json_object(
         'v', 1, 'id', NEW.id,
         'homeRunId', pr.home_run_id, 'peerRunId', pr.run_id,
         'message', json_object(
           'from', NEW.from_handle, 'type', NEW.type, 'subject', NEW.subject,
           'body', NEW.body, 'payload', NEW.payload, 'priority', NEW.priority,
           'threadId', NEW.thread_id
         ),
         'dispatchId', p.dispatch_id, 'taskId', p.task_id
       ) AS payload
     FROM peer_runs pr
     LEFT JOIN message_dispatch_provenance p ON p.message_id = NEW.id
       AND p.dispatch_id IN (SELECT id FROM dispatch_contexts WHERE run_id = pr.run_id)
     WHERE NEW.to_handle = 'run:' || pr.run_id
       AND NOT EXISTS (
         SELECT 1 FROM federation_relay_items
         WHERE dispatch_id = pr.conversation_id AND direction = 'to_home'
           AND message_id = NEW.id
       )); END;
CREATE TRIGGER trg_peer_run_mail_reroute AFTER UPDATE OF to_handle ON messages
      WHEN NEW.to_handle LIKE 'run:%' AND NEW.to_handle <> OLD.to_handle AND NEW.type <> 'heartbeat'
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT pr.conversation_id, NEW.id AS message_id,
       json_object(
         'v', 1, 'id', NEW.id,
         'homeRunId', pr.home_run_id, 'peerRunId', pr.run_id,
         'message', json_object(
           'from', NEW.from_handle, 'type', NEW.type, 'subject', NEW.subject,
           'body', NEW.body, 'payload', NEW.payload, 'priority', NEW.priority,
           'threadId', NEW.thread_id
         ),
         'dispatchId', p.dispatch_id, 'taskId', p.task_id
       ) AS payload
     FROM peer_runs pr
     LEFT JOIN message_dispatch_provenance p ON p.message_id = NEW.id
       AND p.dispatch_id IN (SELECT id FROM dispatch_contexts WHERE run_id = pr.run_id)
     WHERE NEW.to_handle = 'run:' || pr.run_id
       AND NOT EXISTS (
         SELECT 1 FROM federation_relay_items
         WHERE dispatch_id = pr.conversation_id AND direction = 'to_home'
           AND message_id = NEW.id
       )) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_mail', payload, length(payload)
     FROM (SELECT pr.conversation_id, NEW.id AS message_id,
       json_object(
         'v', 1, 'id', NEW.id,
         'homeRunId', pr.home_run_id, 'peerRunId', pr.run_id,
         'message', json_object(
           'from', NEW.from_handle, 'type', NEW.type, 'subject', NEW.subject,
           'body', NEW.body, 'payload', NEW.payload, 'priority', NEW.priority,
           'threadId', NEW.thread_id
         ),
         'dispatchId', p.dispatch_id, 'taskId', p.task_id
       ) AS payload
     FROM peer_runs pr
     LEFT JOIN message_dispatch_provenance p ON p.message_id = NEW.id
       AND p.dispatch_id IN (SELECT id FROM dispatch_contexts WHERE run_id = pr.run_id)
     WHERE NEW.to_handle = 'run:' || pr.run_id
       AND NOT EXISTS (
         SELECT 1 FROM federation_relay_items
         WHERE dispatch_id = pr.conversation_id AND direction = 'to_home'
           AND message_id = NEW.id
       )); END;
CREATE TRIGGER trg_peer_run_dispatch_insert AFTER INSERT ON dispatch_contexts
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.id) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_state', payload, length(payload)
     FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.id); END;
CREATE TRIGGER trg_peer_run_dispatch_status AFTER UPDATE OF status ON dispatch_contexts
      WHEN NEW.status <> OLD.status AND NOT EXISTS (SELECT 1 FROM peer_run_corrections WHERE dispatch_id = NEW.id)
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.id) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_state', payload, length(payload)
     FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.id); END;
CREATE TRIGGER trg_peer_run_worker_state AFTER UPDATE OF state ON worker_dispatches
      WHEN NEW.state <> OLD.state AND NOT EXISTS (SELECT 1 FROM peer_run_corrections WHERE dispatch_id = NEW.dispatch_id)
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.dispatch_id) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_state', payload, length(payload)
     FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = NEW.dispatch_id); END;
CREATE TRIGGER trg_peer_run_task_status AFTER UPDATE OF status ON tasks
      WHEN NEW.status <> OLD.status AND NOT EXISTS (
        SELECT 1 FROM peer_run_corrections c JOIN dispatch_contexts d ON d.id = c.dispatch_id
        WHERE d.task_id = NEW.id
      )
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = (SELECT id FROM dispatch_contexts WHERE task_id = NEW.id ORDER BY rowid DESC LIMIT 1)) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_state', payload, length(payload)
     FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('false'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = (SELECT id FROM dispatch_contexts WHERE task_id = NEW.id ORDER BY rowid DESC LIMIT 1)); END;
CREATE TRIGGER trg_peer_run_correction AFTER DELETE ON peer_run_corrections
      BEGIN INSERT INTO federation_relay_sequences (dispatch_id, direction, last_sequence)
     SELECT conversation_id, 'to_home', 1 FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('true'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = OLD.dispatch_id) WHERE true
     ON CONFLICT(dispatch_id, direction) DO UPDATE SET last_sequence = last_sequence + 1;
INSERT INTO federation_relay_items
       (dispatch_id, direction, sequence, message_id, kind, payload, byte_count)
     SELECT conversation_id, 'to_home', (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'), message_id, 'worker_state', payload, length(payload)
     FROM (SELECT conversation_id, 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home') AS message_id,
       json_object(
         'v', 1, 'id', 'state_' || (SELECT last_sequence FROM federation_relay_sequences
  WHERE dispatch_id = conversation_id AND direction = 'to_home'),
         'homeRunId', home_run_id, 'peerRunId', peer_run_id,
         'dispatch', json(dispatch_json), 'task', json(task_json),
         'correctedUnobservedPrompt', json('true'),
         'removed', NULL
       ) AS payload
     FROM peer_run_dispatch_state WHERE dispatch_id = OLD.dispatch_id); END;
CREATE TRIGGER trg_tasks_record_removed_work
AFTER DELETE ON tasks
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(OLD.run_id AS TEXT), 'task', COALESCE(CAST(OLD.id AS TEXT), ''),
    json_object('v', 1, 'parentTaskId', CAST(OLD.parent_id AS TEXT), 'title', CAST(COALESCE(OLD.task_title, OLD.display_name) AS TEXT), 'createdByHandle', CAST(OLD.created_by_terminal_handle AS TEXT), 'createdByPaneKey', CAST(OLD.created_by_pane_key AS TEXT), 'deps', CAST(OLD.deps AS TEXT), 'status', CAST(OLD.status AS TEXT), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE TRIGGER trg_dispatch_contexts_record_removed_work
AFTER DELETE ON dispatch_contexts
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(OLD.run_id AS TEXT), 'dispatch', COALESCE(CAST(OLD.id AS TEXT), ''),
    json_object('v', 1, 'taskId', CAST(OLD.task_id AS TEXT), 'retryOfDispatchId', CAST(OLD.retry_of_dispatch_id AS TEXT), 'creatorDispatchId', CAST(OLD.creator_dispatch_id AS TEXT), 'creatorHandle', CAST(OLD.creator_handle AS TEXT), 'creatorPaneKey', CAST(OLD.creator_pane_key AS TEXT), 'creatorOrcaSessionId', CAST(OLD.creator_orca_session_id AS TEXT), 'assigneeHandle', CAST(OLD.assignee_handle AS TEXT), 'assigneePaneKey', CAST(OLD.assignee_pane_key AS TEXT), 'assigneeOrcaSessionId', CAST(OLD.assignee_orca_session_id AS TEXT), 'hostScope', CAST(OLD.host_scope AS TEXT), 'depth', CAST(OLD.depth AS INTEGER), 'status', CAST(OLD.status AS TEXT), 'terminationReason', CAST(OLD.termination_reason AS TEXT), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE TRIGGER trg_worker_dispatches_record_removed_work
AFTER DELETE ON worker_dispatches
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(COALESCE(
    (SELECT run_id FROM dispatch_contexts WHERE id = OLD.dispatch_id),
    (SELECT NULLIF(home_run_id, '') FROM remote_dispatch_attachments WHERE dispatch_id = OLD.dispatch_id),
    (SELECT run_id FROM removed_orchestration_work
      WHERE entity_id = OLD.dispatch_id AND kind IN ('dispatch', 'remote_worker')
      ORDER BY seq DESC LIMIT 1)
  ) AS TEXT), 'worker', COALESCE(CAST(OLD.dispatch_id AS TEXT), ''),
    json_object('v', 1, 'state', CAST(OLD.state AS TEXT), 'stage', CAST(OLD.stage AS TEXT), 'worktreeId', CAST(OLD.worktree_id AS TEXT), 'terminalHandle', CAST(OLD.agent_terminal_handle AS TEXT), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE TRIGGER trg_worker_terminal_resources_record_removed_work
AFTER DELETE ON worker_terminal_resources
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(COALESCE(COALESCE(
    (SELECT run_id FROM dispatch_contexts WHERE id = OLD.owner_dispatch_id),
    (SELECT NULLIF(home_run_id, '') FROM remote_dispatch_attachments WHERE dispatch_id = OLD.owner_dispatch_id),
    (SELECT run_id FROM removed_orchestration_work
      WHERE entity_id = OLD.owner_dispatch_id AND kind IN ('dispatch', 'remote_worker')
      ORDER BY seq DESC LIMIT 1)
  ), COALESCE(
    (SELECT run_id FROM dispatch_contexts WHERE id = OLD.origin_dispatch_id),
    (SELECT NULLIF(home_run_id, '') FROM remote_dispatch_attachments WHERE dispatch_id = OLD.origin_dispatch_id),
    (SELECT run_id FROM removed_orchestration_work
      WHERE entity_id = OLD.origin_dispatch_id AND kind IN ('dispatch', 'remote_worker')
      ORDER BY seq DESC LIMIT 1)
  )) AS TEXT), 'terminal', COALESCE(CAST(OLD.id AS TEXT), ''),
    json_object('v', 1, 'originDispatchId', CAST(OLD.origin_dispatch_id AS TEXT), 'ownerDispatchId', CAST(OLD.owner_dispatch_id AS TEXT), 'priorOwnerDispatchIds', CAST(OLD.prior_owner_dispatch_ids AS TEXT), 'terminalHandle', CAST(OLD.terminal_handle AS TEXT), 'paneKey', CAST(OLD.pane_key AS TEXT), 'worktreeId', CAST(OLD.worktree_id AS TEXT), 'hostScope', CAST(OLD.host_scope AS TEXT), 'ownershipState', CAST(OLD.ownership_state AS TEXT), 'releaseState', CAST(OLD.release_state AS TEXT), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE TRIGGER trg_remote_dispatch_attachments_record_removed_work
AFTER DELETE ON remote_dispatch_attachments
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(NULLIF(OLD.home_run_id, '') AS TEXT), 'remote_worker', COALESCE(CAST(OLD.dispatch_id AS TEXT), ''),
    json_object('v', 1, 'taskId', CAST(OLD.task_id AS TEXT), 'homePeerFingerprint', CAST(OLD.home_peer_fingerprint AS TEXT), 'state', CAST(OLD.state AS TEXT), 'stage', CAST(OLD.stage AS TEXT), 'worktreeId', CAST(OLD.worktree_id AS TEXT), 'terminalHandle', CAST(OLD.terminal_handle AS TEXT), 'paneKey', CAST(OLD.pane_key AS TEXT), 'depth', CAST(OLD.depth AS INTEGER), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE TRIGGER trg_federated_dispatches_record_removed_work
AFTER DELETE ON federated_dispatches
BEGIN
  INSERT INTO removed_orchestration_work (run_id, kind, entity_id, record)
  VALUES (CAST(COALESCE(
    (SELECT run_id FROM dispatch_contexts WHERE id = OLD.dispatch_id),
    (SELECT NULLIF(home_run_id, '') FROM remote_dispatch_attachments WHERE dispatch_id = OLD.dispatch_id),
    (SELECT run_id FROM removed_orchestration_work
      WHERE entity_id = OLD.dispatch_id AND kind IN ('dispatch', 'remote_worker')
      ORDER BY seq DESC LIMIT 1)
  ) AS TEXT), 'federated_worker', COALESCE(CAST(OLD.dispatch_id AS TEXT), ''),
    json_object('v', 1, 'environmentId', CAST(OLD.environment_id AS TEXT), 'environmentName', CAST(OLD.environment_name AS TEXT), 'remoteWorktreeId', CAST(OLD.remote_worktree_id AS TEXT), 'remoteTerminalHandle', CAST(OLD.remote_terminal_handle AS TEXT), 'dispatchStatus', CAST((SELECT status FROM dispatch_contexts WHERE id = OLD.dispatch_id) AS TEXT), 'createdAt', CAST(OLD.created_at AS TEXT)));
END;
CREATE INDEX idx_dispatch_assignee_orca_session_backfill
      ON dispatch_contexts(id) WHERE assignee_orca_session_id IS NULL
  AND (process_incarnation GLOB 'structured:*' OR assignee_handle GLOB 'structworker_*');
CREATE INDEX idx_dispatch_creator_orca_session_backfill
      ON dispatch_contexts(id) WHERE creator_orca_session_id IS NULL AND creator_handle GLOB 'structworker_*';
CREATE INDEX idx_dispatch_contexts_run ON dispatch_contexts(run_id);
CREATE INDEX idx_question_threads_run ON question_threads(run_id);
CREATE INDEX idx_worker_terminal_resources_origin
        ON worker_terminal_resources(origin_dispatch_id);
CREATE INDEX idx_worker_terminal_resources_unreleased_minted
        ON worker_terminal_resources(owner_dispatch_id) WHERE release_state = 'not_requested'
  AND ownership_state = 'owned'
  AND process_incarnation LIKE '%:________-____-____-____-____________'
  AND terminal_handle NOT LIKE 'structworker\_%' ESCAPE '\';
CREATE INDEX idx_attempt_observation_facts_task
        ON attempt_observation_facts(task_id);
CREATE INDEX idx_message_dispatch_provenance_dispatch
        ON message_dispatch_provenance(dispatch_id);
CREATE INDEX idx_remote_run_links_run ON remote_run_links(run_id);
CREATE INDEX idx_remote_placements_run ON remote_placements(run_id);
CREATE INDEX idx_worker_dispatches_agent_terminal_handle
        ON worker_dispatches(agent_terminal_handle) WHERE agent_terminal_handle IS NOT NULL;
CREATE INDEX idx_attempt_observation_facts_liveness
        ON attempt_observation_facts(dispatch_id) WHERE facet = 'liveness';
CREATE INDEX idx_dispatch_creator_mailbox_work
        ON dispatch_contexts(creator_dispatch_id, status);
CREATE INDEX idx_runs_current_coordinator_handle ON runs(coordinator_handle);
CREATE INDEX idx_runs_current_pane_exact
        ON runs(coordinator_pane_key) WHERE legacy = 0 AND coordinator_pane_key IS NOT NULL;
CREATE INDEX idx_runs_current_pane_leaf
        ON runs(substr(coordinator_pane_key, instr(coordinator_pane_key, ':') + 1))
        WHERE legacy = 0 AND coordinator_pane_key IS NOT NULL
          AND instr(coordinator_pane_key, ':') > 1;
CREATE INDEX idx_messages_native_account_work
        ON messages(run_id, to_handle, type)
        WHERE read = 0 AND delivered_at IS NULL AND delivery_contract <> 'audit_only';
CREATE INDEX idx_messages_native_direct_work
        ON messages(to_handle, type)
        WHERE read = 0 AND delivered_at IS NULL AND delivery_contract <> 'audit_only';
CREATE INDEX idx_messages_stop_reports
        ON messages(run_id) WHERE id LIKE 'msg\_stop\_%' ESCAPE '\';
CREATE TRIGGER trg_messages_route_coordinator_mail
      AFTER INSERT ON messages
      WHEN NEW.read = 0 AND NEW.delivery_contract = 'current_delivery'
        AND EXISTS (
          SELECT 1 FROM runs
          WHERE runs.id = NEW.run_id AND runs.legacy = 0
        )
        AND EXISTS (
    SELECT 1 FROM run_coordinator_handles AS alias
    WHERE alias.run_id = NEW.run_id AND alias.terminal_handle = NEW.to_handle
      AND NOT (EXISTS (
    SELECT 1 FROM dispatch_contexts AS grab
    WHERE grab.assignee_handle = alias.terminal_handle
      AND +grab.run_id = alias.run_id
      AND NOT (grab.creator_handle IS NOT NULL AND grab.creator_handle = grab.assignee_handle)
      AND julianday(grab.created_at) <= julianday(alias.first_bound_at)
      AND (grab.status IN ('pending', 'dispatched')
        OR julianday(datetime(grab.completed_at)) > julianday(alias.first_bound_at))
  )
    OR EXISTS (
    SELECT 1 FROM dispatch_contexts AS grab
    WHERE grab.assignee_orca_session_id = (CASE WHEN substr(alias.terminal_handle, 1, 16) = 'orca_session_id:'
    THEN substr(alias.terminal_handle, 17) END)
      AND +grab.run_id = alias.run_id
      AND NOT (grab.creator_handle IS NOT NULL AND grab.creator_handle = grab.assignee_handle)
      AND julianday(grab.created_at) <= julianday(alias.first_bound_at)
      AND (grab.status IN ('pending', 'dispatched')
        OR julianday(datetime(grab.completed_at)) > julianday(alias.first_bound_at))
  ))
  )
        AND NOT EXISTS (
    SELECT 1 FROM dispatch_contexts
    WHERE dispatch_contexts.run_id = NEW.run_id
      AND (dispatch_contexts.assignee_handle = NEW.to_handle
        OR ('orca_session_id:' || dispatch_contexts.assignee_orca_session_id) = NEW.to_handle)
      AND dispatch_contexts.status IN ('pending', 'dispatched')
  )
      BEGIN
        UPDATE messages SET to_handle = 'run:' || NEW.run_id WHERE sequence = NEW.sequence;
      END;
CREATE TABLE bot_groups (id TEXT PRIMARY KEY, record TEXT NOT NULL);
CREATE TABLE bot_group_messages (
      sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
      group_id TEXT NOT NULL, record TEXT NOT NULL);
CREATE TABLE bot_group_operations (operation_id TEXT PRIMARY KEY, message_id TEXT NOT NULL);
CREATE TABLE bot_group_pending (operation_id TEXT PRIMARY KEY, group_id TEXT NOT NULL, context TEXT NOT NULL);
CREATE TABLE bot_group_worker_operations (dispatch_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL);
CREATE INDEX bot_group_worker_operation ON bot_group_worker_operations(operation_id);
CREATE INDEX bot_group_messages_room ON bot_group_messages(group_id, sequence);
CREATE INDEX bot_group_messages_thread
      ON bot_group_messages(group_id, coalesce(json_extract(record, '$.threadId'), ''), sequence);
CREATE TABLE bot_group_sidebar (group_id TEXT PRIMARY KEY, sidebar_order INTEGER, latest TEXT);
CREATE TABLE bot_group_remote_asks (request_id TEXT PRIMARY KEY, message_id TEXT NOT NULL);
CREATE INDEX bot_group_remote_asks_message ON bot_group_remote_asks(message_id);
CREATE TABLE bot_group_remote_asks_scanned (id INTEGER PRIMARY KEY CHECK (id = 1), sequence INTEGER NOT NULL);
CREATE TRIGGER bot_groups_keep_remoteMembers AFTER UPDATE OF record ON bot_groups
    WHEN json_type(NEW.record, '$.remoteMembers') IS NULL AND json_type(OLD.record, '$.remoteMembers') IS NOT NULL
    BEGIN
      UPDATE bot_groups SET record = json_set(record, '$.remoteMembers', OLD.record -> '$.remoteMembers') WHERE rowid = NEW.rowid;
    END;
CREATE TRIGGER bot_group_messages_keep_images AFTER UPDATE OF record ON bot_group_messages
    WHEN json_type(NEW.record, '$.images') IS NULL AND json_type(OLD.record, '$.images') IS NOT NULL
    BEGIN
      UPDATE bot_group_messages SET record = json_set(record, '$.images', OLD.record -> '$.images') WHERE rowid = NEW.rowid;
    END;
CREATE TRIGGER bot_group_messages_keep_remoteAsks AFTER UPDATE OF record ON bot_group_messages
    WHEN json_type(NEW.record, '$.remoteAsks') IS NULL AND json_type(OLD.record, '$.remoteAsks') IS NOT NULL
    BEGIN
      UPDATE bot_group_messages SET record = json_set(record, '$.remoteAsks', OLD.record -> '$.remoteAsks') WHERE rowid = NEW.rowid;
    END;
CREATE TRIGGER bot_group_messages_drop_remote_asks AFTER DELETE ON bot_group_messages
          BEGIN DELETE FROM bot_group_remote_asks WHERE message_id = OLD.id; END;
PRAGMA user_version = 50;
