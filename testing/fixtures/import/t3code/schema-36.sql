CREATE TABLE "effect_sql_migrations" (
  migration_id integer PRIMARY KEY NOT NULL,
  created_at datetime NOT NULL DEFAULT current_timestamp,
  name VARCHAR(255) NOT NULL
);
CREATE TABLE orchestration_events (
      sequence INTEGER PRIMARY KEY AUTOINCREMENT,
      event_id TEXT NOT NULL UNIQUE,
      aggregate_kind TEXT NOT NULL,
      stream_id TEXT NOT NULL,
      stream_version INTEGER NOT NULL,
      event_type TEXT NOT NULL,
      occurred_at TEXT NOT NULL,
      command_id TEXT,
      causation_event_id TEXT,
      correlation_id TEXT,
      actor_kind TEXT NOT NULL,
      payload_json TEXT NOT NULL,
      metadata_json TEXT NOT NULL
    );
CREATE TABLE orchestration_command_receipts (
      command_id TEXT PRIMARY KEY,
      aggregate_kind TEXT NOT NULL,
      aggregate_id TEXT NOT NULL,
      accepted_at TEXT NOT NULL,
      result_sequence INTEGER NOT NULL,
      status TEXT NOT NULL,
      error TEXT
    );
CREATE TABLE checkpoint_diff_blobs (
      thread_id TEXT NOT NULL,
      from_turn_count INTEGER NOT NULL,
      to_turn_count INTEGER NOT NULL,
      diff TEXT NOT NULL,
      created_at TEXT NOT NULL,
      UNIQUE (thread_id, from_turn_count, to_turn_count)
    );
CREATE TABLE provider_session_runtime (
      thread_id TEXT PRIMARY KEY,
      provider_name TEXT NOT NULL,
      adapter_key TEXT NOT NULL,
      runtime_mode TEXT NOT NULL DEFAULT 'full-access',
      status TEXT NOT NULL,
      last_seen_at TEXT NOT NULL,
      resume_cursor_json TEXT,
      runtime_payload_json TEXT
    , provider_instance_id TEXT);
CREATE TABLE projection_projects (
      project_id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      workspace_root TEXT NOT NULL,
      scripts_json TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      deleted_at TEXT
    , default_model_selection_json TEXT);
CREATE TABLE projection_threads (
      thread_id TEXT PRIMARY KEY,
      project_id TEXT NOT NULL,
      title TEXT NOT NULL,
      branch TEXT,
      worktree_path TEXT,
      latest_turn_id TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      deleted_at TEXT
    , runtime_mode TEXT NOT NULL DEFAULT 'full-access', interaction_mode TEXT NOT NULL DEFAULT 'default', model_selection_json TEXT, archived_at TEXT, latest_user_message_at TEXT, pending_approval_count INTEGER NOT NULL DEFAULT 0, pending_user_input_count INTEGER NOT NULL DEFAULT 0, has_actionable_proposed_plan INTEGER NOT NULL DEFAULT 0, settled_override TEXT, settled_at TEXT, snoozed_until TEXT, snoozed_at TEXT, title_regeneration_request_id TEXT, title_regeneration_started_at TEXT, pinned_at TEXT);
CREATE TABLE projection_thread_messages (
      message_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      role TEXT NOT NULL,
      text TEXT NOT NULL,
      is_streaming INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    , attachments_json TEXT);
CREATE TABLE projection_thread_activities (
      activity_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      tone TEXT NOT NULL,
      kind TEXT NOT NULL,
      summary TEXT NOT NULL,
      payload_json TEXT NOT NULL,
      created_at TEXT NOT NULL
    , sequence INTEGER);
CREATE TABLE projection_thread_sessions (
      thread_id TEXT PRIMARY KEY,
      status TEXT NOT NULL,
      provider_name TEXT,
      provider_session_id TEXT,
      provider_thread_id TEXT,
      active_turn_id TEXT,
      last_error TEXT,
      updated_at TEXT NOT NULL
    , runtime_mode TEXT NOT NULL DEFAULT 'full-access', provider_instance_id TEXT);
CREATE TABLE projection_turns (
      row_id INTEGER PRIMARY KEY AUTOINCREMENT,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      pending_message_id TEXT,
      assistant_message_id TEXT,
      state TEXT NOT NULL,
      requested_at TEXT NOT NULL,
      started_at TEXT,
      completed_at TEXT,
      checkpoint_turn_count INTEGER,
      checkpoint_ref TEXT,
      checkpoint_status TEXT,
      checkpoint_files_json TEXT NOT NULL, source_proposed_plan_thread_id TEXT, source_proposed_plan_id TEXT,
      UNIQUE (thread_id, turn_id),
      UNIQUE (thread_id, checkpoint_turn_count)
    );
CREATE TABLE projection_pending_approvals (
      request_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      status TEXT NOT NULL,
      decision TEXT,
      created_at TEXT NOT NULL,
      resolved_at TEXT
    );
CREATE TABLE projection_state (
      projector TEXT PRIMARY KEY,
      last_applied_sequence INTEGER NOT NULL,
      updated_at TEXT NOT NULL
    );
CREATE TABLE projection_thread_proposed_plans (
      plan_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      plan_markdown TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    , implemented_at TEXT, implementation_thread_id TEXT);
CREATE TABLE auth_pairing_links (
      id TEXT PRIMARY KEY,
      credential TEXT NOT NULL UNIQUE,
      method TEXT NOT NULL,
      scopes TEXT NOT NULL,
      subject TEXT NOT NULL,
      label TEXT,
      created_at TEXT NOT NULL,
      expires_at TEXT NOT NULL,
      consumed_at TEXT,
      revoked_at TEXT
    , proof_key_thumbprint TEXT);
CREATE TABLE auth_sessions (
      session_id TEXT PRIMARY KEY,
      subject TEXT NOT NULL,
      scopes TEXT NOT NULL,
      method TEXT NOT NULL,
      client_label TEXT,
      client_ip_address TEXT,
      client_user_agent TEXT,
      client_device_type TEXT NOT NULL DEFAULT 'unknown',
      client_os TEXT,
      client_browser TEXT,
      issued_at TEXT NOT NULL,
      expires_at TEXT NOT NULL,
      last_connected_at TEXT,
      revoked_at TEXT
    );
CREATE UNIQUE INDEX idx_orch_events_stream_version
    ON orchestration_events(aggregate_kind, stream_id, stream_version);
CREATE INDEX idx_orch_events_stream_sequence
    ON orchestration_events(aggregate_kind, stream_id, sequence);
CREATE INDEX idx_orch_events_command_id
    ON orchestration_events(command_id);
CREATE INDEX idx_orch_events_correlation_id
    ON orchestration_events(correlation_id);
CREATE INDEX idx_orch_command_receipts_aggregate
    ON orchestration_command_receipts(aggregate_kind, aggregate_id);
CREATE INDEX idx_orch_command_receipts_sequence
    ON orchestration_command_receipts(result_sequence);
CREATE INDEX idx_checkpoint_diff_blobs_thread_to_turn
    ON checkpoint_diff_blobs(thread_id, to_turn_count);
CREATE INDEX idx_provider_session_runtime_status
    ON provider_session_runtime(status);
CREATE INDEX idx_provider_session_runtime_provider
    ON provider_session_runtime(provider_name);
CREATE INDEX idx_projection_projects_updated_at
    ON projection_projects(updated_at);
CREATE INDEX idx_projection_threads_project_id
    ON projection_threads(project_id);
CREATE INDEX idx_projection_thread_messages_thread_created
    ON projection_thread_messages(thread_id, created_at);
CREATE INDEX idx_projection_thread_activities_thread_created
    ON projection_thread_activities(thread_id, created_at);
CREATE INDEX idx_projection_thread_sessions_provider_session
    ON projection_thread_sessions(provider_session_id);
CREATE INDEX idx_projection_turns_thread_requested
    ON projection_turns(thread_id, requested_at);
CREATE INDEX idx_projection_turns_thread_checkpoint_completed
    ON projection_turns(thread_id, checkpoint_turn_count, completed_at);
CREATE INDEX idx_projection_pending_approvals_thread_status
    ON projection_pending_approvals(thread_id, status);
CREATE INDEX idx_projection_thread_activities_thread_sequence
    ON projection_thread_activities(thread_id, sequence);
CREATE INDEX idx_projection_thread_proposed_plans_thread_created
    ON projection_thread_proposed_plans(thread_id, created_at);
CREATE INDEX idx_projection_threads_project_archived_at
    ON projection_threads(project_id, archived_at);
CREATE INDEX idx_projection_projects_workspace_root_deleted_at
    ON projection_projects(workspace_root, deleted_at);
CREATE INDEX idx_projection_threads_project_deleted_created
    ON projection_threads(project_id, deleted_at, created_at);
CREATE INDEX idx_provider_session_runtime_instance
    ON provider_session_runtime(provider_instance_id);
CREATE INDEX idx_projection_thread_sessions_instance
    ON projection_thread_sessions(provider_instance_id);
CREATE INDEX idx_projection_thread_activities_thread_sequence_created_id
    ON projection_thread_activities(thread_id, sequence, created_at, activity_id);
CREATE INDEX idx_projection_thread_messages_thread_created_id
    ON projection_thread_messages(thread_id, created_at, message_id);
CREATE INDEX idx_projection_threads_shell_active
    ON projection_threads(deleted_at, archived_at, project_id, created_at, thread_id);
CREATE INDEX idx_projection_threads_shell_archived
    ON projection_threads(deleted_at, archived_at, project_id, thread_id);
CREATE INDEX idx_auth_pairing_links_active
    ON auth_pairing_links(revoked_at, consumed_at, expires_at);
CREATE INDEX idx_auth_sessions_active
    ON auth_sessions(revoked_at, expires_at, issued_at);
