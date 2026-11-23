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
    , application_event_version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE orchestration_command_receipts (
      command_id TEXT PRIMARY KEY,
      aggregate_kind TEXT NOT NULL,
      aggregate_id TEXT NOT NULL,
      accepted_at TEXT NOT NULL,
      result_sequence INTEGER NOT NULL,
      status TEXT NOT NULL,
      error TEXT
    , command_type TEXT NOT NULL DEFAULT 'legacy');
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
    , default_model_selection_json TEXT, default_thread_env_mode TEXT, favicon_path TEXT, auto_pull INTEGER NOT NULL DEFAULT 0, project_icon_json TEXT, pinned_at TEXT, archived_at TEXT, has_custom_title INTEGER NOT NULL DEFAULT 0);
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
    , runtime_mode TEXT NOT NULL DEFAULT 'full-access', interaction_mode TEXT NOT NULL DEFAULT 'default', model_selection_json TEXT, archived_at TEXT, latest_user_message_at TEXT, pending_approval_count INTEGER NOT NULL DEFAULT 0, pending_user_input_count INTEGER NOT NULL DEFAULT 0, has_actionable_proposed_plan INTEGER NOT NULL DEFAULT 0, settled_override TEXT, settled_at TEXT, snoozed_until TEXT, snoozed_at TEXT, title_regeneration_request_id TEXT, title_regeneration_started_at TEXT, pinned_at TEXT, pin_order_key TEXT, linked_pull_request_json TEXT, unsettled_at TEXT, branch_pull_request_json TEXT, active_order_key TEXT, title_state_json TEXT, auto_settle_disabled_at TEXT);
CREATE TABLE projection_thread_messages (
      message_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      turn_id TEXT,
      role TEXT NOT NULL,
      text TEXT NOT NULL,
      is_streaming INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    , attachments_json TEXT, context_json TEXT);
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
    , proof_key_thumbprint TEXT, requires_connect_approval INTEGER NOT NULL DEFAULT 0, requested_client_json TEXT, connect_denied INTEGER NOT NULL DEFAULT 0);
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
    , client_surface TEXT, client_app_version TEXT, proof_key_thumbprint TEXT);
CREATE TABLE projection_thread_pull_requests (
      thread_id TEXT NOT NULL,
      host TEXT NOT NULL,
      repository TEXT NOT NULL,
      number INTEGER NOT NULL,
      url TEXT NOT NULL,
      source TEXT NOT NULL,
      linked_at TEXT NOT NULL,
      snapshot_json TEXT,
      stack_json TEXT,
      PRIMARY KEY (thread_id, host, repository, number)
    );
CREATE TABLE pull_request_files_viewed (
      provider TEXT NOT NULL,
      host TEXT NOT NULL,
      repository TEXT NOT NULL,
      number INTEGER NOT NULL,
      viewer TEXT NOT NULL,
      path TEXT NOT NULL,
      revision TEXT,
      viewed_at TEXT NOT NULL,
      PRIMARY KEY (provider, host, repository, number, viewer, path)
    ) WITHOUT ROWID;
CREATE TABLE orchestration_v2_events (
      sequence INTEGER PRIMARY KEY AUTOINCREMENT,
      event_id TEXT NOT NULL UNIQUE,
      command_id TEXT,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT,
      provider TEXT,
      raw_event_id TEXT,
      event_type TEXT NOT NULL,
      occurred_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , driver TEXT, provider_instance_id TEXT);
CREATE TABLE orchestration_v2_command_receipts (
      command_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      command_type TEXT NOT NULL,
      accepted_at TEXT NOT NULL,
      result_sequence INTEGER NOT NULL,
      status TEXT NOT NULL,
      error TEXT
    );
CREATE TABLE orchestration_v2_projection_threads (
      thread_id TEXT PRIMARY KEY,
      project_id TEXT NOT NULL,
      title TEXT NOT NULL,
      default_provider TEXT NOT NULL,
      runtime_mode TEXT NOT NULL,
      interaction_mode TEXT NOT NULL,
      active_provider_thread_id TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      archived_at TEXT,
      deleted_at TEXT,
      payload_json TEXT NOT NULL
    , provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_runs (
      run_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      ordinal INTEGER NOT NULL,
      provider TEXT NOT NULL,
      provider_thread_id TEXT,
      status TEXT NOT NULL,
      requested_at TEXT NOT NULL,
      completed_at TEXT,
      payload_json TEXT NOT NULL
    , provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_run_attempts (
      attempt_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT NOT NULL,
      attempt_ordinal INTEGER NOT NULL,
      root_node_id TEXT NOT NULL,
      provider TEXT NOT NULL,
      provider_thread_id TEXT NOT NULL,
      provider_turn_id TEXT,
      status TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_nodes (
      node_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      parent_node_id TEXT,
      root_node_id TEXT NOT NULL,
      kind TEXT NOT NULL,
      status TEXT NOT NULL,
      provider_thread_id TEXT,
      provider_turn_id TEXT,
      runtime_request_id TEXT,
      checkpoint_scope_id TEXT,
      started_at TEXT,
      completed_at TEXT,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_provider_sessions (
      provider_session_id TEXT PRIMARY KEY,
      thread_id TEXT,
      provider TEXT NOT NULL,
      status TEXT NOT NULL,
      model TEXT,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , driver TEXT, provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_provider_threads (
      provider_thread_id TEXT PRIMARY KEY,
      thread_id TEXT,
      owner_node_id TEXT,
      provider TEXT NOT NULL,
      provider_session_id TEXT,
      status TEXT NOT NULL,
      first_run_ordinal INTEGER,
      last_run_ordinal INTEGER,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , driver TEXT, provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_provider_turns (
      provider_turn_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      provider_thread_id TEXT NOT NULL,
      node_id TEXT NOT NULL,
      run_attempt_id TEXT,
      ordinal INTEGER NOT NULL,
      status TEXT NOT NULL,
      started_at TEXT,
      completed_at TEXT,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_runtime_requests (
      runtime_request_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      node_id TEXT NOT NULL,
      provider_turn_id TEXT,
      kind TEXT NOT NULL,
      status TEXT NOT NULL,
      created_at TEXT NOT NULL,
      resolved_at TEXT,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_messages (
      message_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT,
      role TEXT NOT NULL,
      streaming INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , search_text TEXT);
CREATE TABLE orchestration_v2_projection_plans (
      plan_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT NOT NULL,
      kind TEXT NOT NULL,
      status TEXT NOT NULL,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_turn_items (
      turn_item_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT,
      provider_thread_id TEXT,
      provider_turn_id TEXT,
      parent_item_id TEXT,
      ordinal INTEGER NOT NULL,
      type TEXT NOT NULL,
      status TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , inline_work_output INTEGER NOT NULL DEFAULT 0);
CREATE TABLE orchestration_v2_projection_checkpoint_scopes (
      scope_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT NOT NULL,
      parent_scope_id TEXT,
      provider_thread_id TEXT,
      kind TEXT NOT NULL,
      ordinal_within_parent INTEGER NOT NULL,
      advances_app_run_count INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_checkpoints (
      checkpoint_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      scope_id TEXT NOT NULL,
      run_id TEXT,
      node_id TEXT NOT NULL,
      parent_checkpoint_id TEXT,
      ordinal_within_scope INTEGER NOT NULL,
      app_run_ordinal INTEGER,
      status TEXT NOT NULL,
      captured_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_context_handoffs (
      context_handoff_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      target_run_id TEXT NOT NULL,
      to_provider_thread_id TEXT NOT NULL,
      strategy TEXT NOT NULL,
      status TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_context_transfers (
      context_transfer_id TEXT PRIMARY KEY,
      source_thread_id TEXT NOT NULL,
      target_thread_id TEXT NOT NULL,
      target_run_id TEXT,
      type TEXT NOT NULL,
      status TEXT NOT NULL,
      source_provider TEXT,
      target_provider TEXT,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , source_provider_instance_id TEXT, target_provider_instance_id TEXT);
CREATE TABLE orchestration_v2_projection_subagents (
      subagent_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      run_id TEXT,
      parent_node_id TEXT NOT NULL,
      provider TEXT NOT NULL,
      provider_thread_id TEXT,
      child_thread_id TEXT,
      origin TEXT NOT NULL,
      status TEXT NOT NULL,
      started_at TEXT,
      completed_at TEXT,
      updated_at TEXT NOT NULL,
      payload_json TEXT NOT NULL
    , driver TEXT, provider_instance_id TEXT, completion_delivery_state TEXT);
CREATE TABLE orchestration_v2_turn_item_positions (
      thread_id TEXT NOT NULL,
      turn_item_id TEXT NOT NULL,
      ordinal INTEGER NOT NULL,
      PRIMARY KEY (thread_id, turn_item_id),
      UNIQUE (thread_id, ordinal)
    );
CREATE TABLE orchestration_v2_projection_metadata (
      projection_name TEXT PRIMARY KEY,
      schema_version INTEGER NOT NULL,
      last_sequence INTEGER NOT NULL,
      updated_at TEXT NOT NULL
    );
CREATE TABLE orchestration_v2_projection_provider_session_bindings (
      provider_session_id TEXT NOT NULL,
      thread_id TEXT NOT NULL,
      PRIMARY KEY (provider_session_id, thread_id)
    );
CREATE TABLE orchestration_v2_thread_launch_workflows (
      command_id TEXT PRIMARY KEY,
      thread_id TEXT NOT NULL,
      project_id TEXT NOT NULL,
      status TEXT NOT NULL,
      title TEXT NOT NULL,
      worktree_path TEXT,
      branch TEXT,
      setup_committed INTEGER NOT NULL DEFAULT 0,
      thread_committed INTEGER NOT NULL DEFAULT 0,
      message_committed INTEGER NOT NULL DEFAULT 0,
      last_error TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
CREATE TABLE "orchestration_v2_effect_outbox" (
      effect_id TEXT PRIMARY KEY,
      command_id TEXT NOT NULL,
      thread_id TEXT NOT NULL,
      effect_type TEXT NOT NULL,
      payload_json TEXT NOT NULL,
      status TEXT NOT NULL CHECK (
        status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled')
      ),
      attempt_count INTEGER NOT NULL DEFAULT 0,
      available_at TEXT NOT NULL,
      lease_owner TEXT,
      lease_expires_at TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      completed_at TEXT,
      last_error TEXT
    );
CREATE TABLE scheduled_tasks (
      task_id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      prompt TEXT NOT NULL,
      enabled INTEGER NOT NULL,
      schedule_json TEXT NOT NULL,
      project_id TEXT NOT NULL,
      thread_id TEXT,
      workspace_strategy_json TEXT NOT NULL,
      model_selection_json TEXT NOT NULL,
      runtime_mode TEXT NOT NULL,
      interaction_mode TEXT NOT NULL,
      created_by TEXT NOT NULL,
      creation_source TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      next_run_at TEXT,
      last_run_at TEXT,
      last_run_status TEXT NOT NULL,
      last_run_error TEXT,
      run_count INTEGER NOT NULL
    , webhook_token TEXT, webhook_secret TEXT, last_run_webhook_delivery_id TEXT, thread_mode TEXT NOT NULL DEFAULT 'new_thread');
CREATE TABLE orchestration_v2_legacy_imports (
      thread_id TEXT PRIMARY KEY,
      source_updated_at TEXT NOT NULL,
      shell_imported_at TEXT NOT NULL,
      transcript_imported_at TEXT,
      imported_message_count INTEGER NOT NULL DEFAULT 0,
      last_error TEXT
    );
CREATE TABLE scheduled_task_webhook_deliveries (
      delivery_id TEXT PRIMARY KEY,
      task_id TEXT NOT NULL,
      received_at TEXT NOT NULL,
      method TEXT NOT NULL,
      query TEXT NOT NULL,
      headers_json TEXT NOT NULL,
      body TEXT NOT NULL,
      body_bytes INTEGER NOT NULL,
      body_truncated INTEGER NOT NULL,
      outcome TEXT NOT NULL,
      signature_verified INTEGER NOT NULL,
      missing_fields_json TEXT NOT NULL,
      rendered_prompt TEXT,
      error TEXT
    );
CREATE TABLE scheduled_task_webhook_relay_deliveries (
      relay_delivery_id TEXT PRIMARY KEY,
      task_id TEXT NOT NULL,
      seen_at TEXT NOT NULL
    );
CREATE TABLE mcp_app_model_context (
      thread_id TEXT NOT NULL,
      item_id TEXT NOT NULL,
      server TEXT NOT NULL,
      tool TEXT NOT NULL,
      text TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      PRIMARY KEY (thread_id, item_id)
    );
CREATE TABLE pull_request_review_guides (
      host TEXT NOT NULL,
      repository TEXT NOT NULL,
      number INTEGER NOT NULL,
      guide_json TEXT NOT NULL,
      PRIMARY KEY (host, repository, number)
    ) WITHOUT ROWID;
CREATE TABLE scheduled_task_webhook_replay_keys (
    task_id TEXT NOT NULL REFERENCES scheduled_tasks(task_id) ON DELETE CASCADE,
    replay_key TEXT NOT NULL,
    delivery_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, replay_key)
  );
CREATE TABLE scheduled_task_webhook_pending (
    delivery_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES scheduled_tasks(task_id) ON DELETE CASCADE,
    coalesce_key TEXT NOT NULL,
    due_at INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    task_json TEXT NOT NULL,
    request_json TEXT NOT NULL,
    delivery_ids_json TEXT NOT NULL
  );
CREATE VIRTUAL TABLE orchestration_v2_message_search USING fts5(
      text,
      content = '',
      contentless_delete = 1,
      tokenize = 'trigram remove_diacritics 1'
    );
CREATE TABLE peer_environment_links (
      environment_id TEXT PRIMARY KEY,
      secret_name TEXT NOT NULL,
      label TEXT NOT NULL,
      urls_json TEXT NOT NULL,
      access TEXT NOT NULL,
      linked_at TEXT NOT NULL,
      expires_at TEXT NOT NULL,
      last_reached_at TEXT,
      last_error TEXT
    , fenced INTEGER NOT NULL DEFAULT 0);
CREATE TABLE orchestration_v2_projection_turn_item_counts (
      thread_id TEXT NOT NULL,
      run_key TEXT NOT NULL,
      item_count INTEGER NOT NULL,
      PRIMARY KEY (thread_id, run_key)
    ) WITHOUT ROWID;
CREATE TABLE auth_connect_devices (
      proof_key_thumbprint TEXT PRIMARY KEY,
      client_json TEXT,
      approved_at TEXT NOT NULL
    );
CREATE UNIQUE INDEX idx_orch_events_stream_version
    ON orchestration_events(aggregate_kind, stream_id, stream_version);
CREATE INDEX idx_orch_events_stream_sequence
    ON orchestration_events(aggregate_kind, stream_id, sequence);
CREATE INDEX idx_orch_events_command_id
    ON orchestration_events(command_id);
CREATE INDEX idx_orch_command_receipts_aggregate
    ON orchestration_command_receipts(aggregate_kind, aggregate_id);
CREATE INDEX idx_orch_command_receipts_sequence
    ON orchestration_command_receipts(result_sequence);
CREATE INDEX idx_provider_session_runtime_status
    ON provider_session_runtime(status);
CREATE INDEX idx_provider_session_runtime_provider
    ON provider_session_runtime(provider_name);
CREATE INDEX idx_projection_projects_updated_at
    ON projection_projects(updated_at);
CREATE INDEX idx_projection_thread_sessions_provider_session
    ON projection_thread_sessions(provider_session_id);
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
CREATE INDEX idx_projection_thread_pull_requests_pr
    ON projection_thread_pull_requests(host, repository, number);
CREATE INDEX orchestration_v2_events_command_idx ON orchestration_v2_events(command_id, sequence);
CREATE INDEX orchestration_v2_events_thread_sequence_idx ON orchestration_v2_events(thread_id, sequence);
CREATE INDEX orchestration_v2_events_thread_type_sequence_idx ON orchestration_v2_events(thread_id, event_type, sequence);
CREATE INDEX orchestration_v2_events_run_sequence_idx ON orchestration_v2_events(run_id, sequence);
CREATE INDEX orchestration_v2_events_node_sequence_idx ON orchestration_v2_events(node_id, sequence);
CREATE INDEX orchestration_v2_events_raw_event_idx ON orchestration_v2_events(raw_event_id);
CREATE INDEX orchestration_v2_command_receipts_thread_sequence_idx ON orchestration_v2_command_receipts(thread_id, result_sequence);
CREATE INDEX orchestration_v2_projection_threads_project_updated_idx ON orchestration_v2_projection_threads(project_id, updated_at);
CREATE UNIQUE INDEX orchestration_v2_projection_runs_thread_ordinal_idx ON orchestration_v2_projection_runs(thread_id, ordinal);
CREATE INDEX orchestration_v2_projection_runs_provider_thread_idx ON orchestration_v2_projection_runs(provider_thread_id);
CREATE INDEX orchestration_v2_projection_runs_thread_status_idx ON orchestration_v2_projection_runs(thread_id, status);
CREATE UNIQUE INDEX orchestration_v2_projection_run_attempts_run_ordinal_idx ON orchestration_v2_projection_run_attempts(run_id, attempt_ordinal);
CREATE INDEX orchestration_v2_projection_run_attempts_thread_idx ON orchestration_v2_projection_run_attempts(thread_id, run_id);
CREATE INDEX orchestration_v2_projection_nodes_thread_run_idx ON orchestration_v2_projection_nodes(thread_id, run_id);
CREATE INDEX orchestration_v2_projection_nodes_parent_idx ON orchestration_v2_projection_nodes(parent_node_id);
CREATE INDEX orchestration_v2_projection_nodes_provider_turn_idx ON orchestration_v2_projection_nodes(provider_turn_id);
CREATE INDEX orchestration_v2_projection_provider_sessions_thread_idx ON orchestration_v2_projection_provider_sessions(thread_id);
CREATE INDEX orchestration_v2_projection_provider_sessions_provider_status_idx ON orchestration_v2_projection_provider_sessions(provider, status);
CREATE INDEX orchestration_v2_projection_provider_threads_thread_idx ON orchestration_v2_projection_provider_threads(thread_id);
CREATE INDEX orchestration_v2_projection_provider_threads_session_idx ON orchestration_v2_projection_provider_threads(provider_session_id);
CREATE INDEX orchestration_v2_projection_provider_threads_owner_idx ON orchestration_v2_projection_provider_threads(owner_node_id);
CREATE INDEX orchestration_v2_projection_provider_turns_thread_idx ON orchestration_v2_projection_provider_turns(thread_id);
CREATE UNIQUE INDEX orchestration_v2_projection_provider_turns_thread_ordinal_idx ON orchestration_v2_projection_provider_turns(provider_thread_id, ordinal);
CREATE INDEX orchestration_v2_projection_runtime_requests_thread_status_idx ON orchestration_v2_projection_runtime_requests(thread_id, status);
CREATE INDEX orchestration_v2_projection_runtime_requests_provider_turn_idx ON orchestration_v2_projection_runtime_requests(provider_turn_id);
CREATE INDEX orchestration_v2_projection_messages_thread_created_idx ON orchestration_v2_projection_messages(thread_id, created_at, message_id);
CREATE INDEX orchestration_v2_projection_messages_run_idx ON orchestration_v2_projection_messages(run_id);
CREATE INDEX orchestration_v2_projection_messages_node_idx ON orchestration_v2_projection_messages(node_id);
CREATE INDEX orchestration_v2_projection_plans_thread_idx ON orchestration_v2_projection_plans(thread_id);
CREATE INDEX orchestration_v2_projection_plans_run_idx ON orchestration_v2_projection_plans(run_id);
CREATE INDEX orchestration_v2_projection_turn_items_thread_ordinal_idx ON orchestration_v2_projection_turn_items(thread_id, ordinal, turn_item_id);
CREATE INDEX orchestration_v2_projection_turn_items_run_ordinal_idx ON orchestration_v2_projection_turn_items(run_id, ordinal);
CREATE INDEX orchestration_v2_projection_turn_items_node_ordinal_idx ON orchestration_v2_projection_turn_items(node_id, ordinal);
CREATE INDEX orchestration_v2_projection_turn_items_provider_turn_idx ON orchestration_v2_projection_turn_items(provider_turn_id);
CREATE INDEX orchestration_v2_projection_checkpoint_scopes_thread_idx ON orchestration_v2_projection_checkpoint_scopes(thread_id);
CREATE INDEX orchestration_v2_projection_checkpoint_scopes_parent_idx ON orchestration_v2_projection_checkpoint_scopes(parent_scope_id);
CREATE UNIQUE INDEX orchestration_v2_projection_checkpoints_scope_ordinal_idx ON orchestration_v2_projection_checkpoints(scope_id, ordinal_within_scope);
CREATE INDEX orchestration_v2_projection_checkpoints_thread_idx ON orchestration_v2_projection_checkpoints(thread_id);
CREATE INDEX orchestration_v2_projection_checkpoints_parent_idx ON orchestration_v2_projection_checkpoints(parent_checkpoint_id);
CREATE INDEX orchestration_v2_projection_context_handoffs_thread_idx ON orchestration_v2_projection_context_handoffs(thread_id);
CREATE INDEX orchestration_v2_projection_context_handoffs_target_run_idx ON orchestration_v2_projection_context_handoffs(target_run_id);
CREATE INDEX orchestration_v2_projection_context_transfers_target_thread_idx ON orchestration_v2_projection_context_transfers(target_thread_id, status);
CREATE INDEX orchestration_v2_projection_context_transfers_target_run_idx ON orchestration_v2_projection_context_transfers(target_run_id);
CREATE INDEX orchestration_v2_projection_subagents_thread_idx ON orchestration_v2_projection_subagents(thread_id, started_at, subagent_id);
CREATE INDEX orchestration_v2_projection_subagents_parent_node_idx ON orchestration_v2_projection_subagents(parent_node_id);
CREATE INDEX orchestration_v2_projection_subagents_provider_thread_idx ON orchestration_v2_projection_subagents(provider_thread_id);
CREATE INDEX orchestration_v2_projection_subagents_child_thread_idx ON orchestration_v2_projection_subagents(child_thread_id);
CREATE INDEX orchestration_v2_events_instance_sequence_idx ON orchestration_v2_events(provider_instance_id, sequence);
CREATE INDEX orchestration_v2_projection_provider_sessions_instance_status_idx ON orchestration_v2_projection_provider_sessions(provider_instance_id, status);
CREATE INDEX orchestration_v2_projection_provider_threads_instance_status_idx ON orchestration_v2_projection_provider_threads(provider_instance_id, status);
CREATE INDEX orchestration_v2_projection_provider_session_bindings_thread_idx
    ON orchestration_v2_projection_provider_session_bindings(thread_id);
CREATE INDEX idx_orchestration_events_application_sequence
    ON orchestration_events(application_event_version, sequence);
CREATE INDEX orchestration_v2_effect_outbox_claim_idx
    ON orchestration_v2_effect_outbox(status, available_at, lease_expires_at, created_at);
CREATE INDEX orchestration_v2_effect_outbox_command_idx
    ON orchestration_v2_effect_outbox(command_id, effect_id);
CREATE INDEX orchestration_v2_effect_outbox_thread_status_idx
    ON orchestration_v2_effect_outbox(thread_id, status, effect_type);
CREATE INDEX idx_scheduled_tasks_due
    ON scheduled_tasks(enabled, next_run_at)
    WHERE enabled = 1 AND next_run_at IS NOT NULL;
CREATE INDEX idx_scheduled_tasks_project
    ON scheduled_tasks(project_id, updated_at);
CREATE INDEX orchestration_v2_legacy_imports_pending_transcript_idx
    ON orchestration_v2_legacy_imports(transcript_imported_at, shell_imported_at, thread_id);
CREATE INDEX idx_orchestration_events_application_high_water
    ON orchestration_events(sequence)
    WHERE aggregate_kind = 'project'
      OR (application_event_version = 2 AND aggregate_kind = 'thread');
CREATE INDEX idx_orchestration_events_agent_stream_sequence
    ON orchestration_events(stream_id, sequence)
    WHERE application_event_version = 2 AND aggregate_kind = 'thread';
CREATE INDEX orchestration_events_v2_created_threads_idx
    ON orchestration_events(stream_id)
    WHERE application_event_version = 2
      AND aggregate_kind = 'thread'
      AND event_type = 'thread.created';
CREATE INDEX orchestration_v2_projection_runs_recovery_idx
    ON orchestration_v2_projection_runs(status, thread_id)
    WHERE status IN ('queued', 'preparing', 'starting', 'running', 'waiting');
CREATE INDEX orchestration_v2_projection_requests_recovery_idx
    ON orchestration_v2_projection_runtime_requests(thread_id)
    WHERE status = 'pending';
CREATE INDEX orchestration_v2_projection_turn_items_recovery_idx
    ON orchestration_v2_projection_turn_items(thread_id)
    WHERE type IN ('command_execution', 'dynamic_tool', 'subagent')
      AND status IN ('pending', 'running', 'waiting');
CREATE INDEX orchestration_v2_projection_turn_items_thread_run_idx
    ON orchestration_v2_projection_turn_items(thread_id, run_id);
CREATE INDEX orchestration_v2_projection_turn_items_shell_pending_idx
    ON orchestration_v2_projection_turn_items(thread_id, run_id)
    WHERE type IN ('command_execution', 'dynamic_tool', 'subagent')
      AND status NOT IN ('completed', 'interrupted', 'failed', 'cancelled');
CREATE INDEX orchestration_v2_projection_messages_latest_user_idx
    ON orchestration_v2_projection_messages(thread_id, updated_at DESC, message_id DESC)
    WHERE role = 'user';
CREATE INDEX idx_scheduled_task_webhook_deliveries_task
    ON scheduled_task_webhook_deliveries(task_id, received_at);
CREATE INDEX idx_scheduled_task_webhook_relay_deliveries_seen
    ON scheduled_task_webhook_relay_deliveries(seen_at);
CREATE INDEX orchestration_v2_projection_turn_items_user_message_idx
    ON orchestration_v2_projection_turn_items(thread_id, ordinal, turn_item_id)
    WHERE type = 'user_message';
CREATE INDEX orchestration_v2_projection_nodes_live_idx
    ON orchestration_v2_projection_nodes(thread_id)
    WHERE status IN ('pending', 'starting', 'running', 'waiting');
CREATE INDEX orchestration_v2_projection_turn_items_failed_error_idx
    ON orchestration_v2_projection_turn_items(
      thread_id, run_id, node_id, updated_at DESC, ordinal DESC, turn_item_id DESC
    )
    WHERE type = 'error' AND status = 'failed';
CREATE INDEX orchestration_v2_provider_turns_codex_native_id_idx
    ON orchestration_v2_projection_provider_turns (
      json_extract(payload_json, '$.nativeTurnRef.nativeId')
    )
    WHERE json_extract(payload_json, '$.nativeTurnRef.driver') = 'codex';
CREATE INDEX idx_webhook_replay_expiry ON scheduled_task_webhook_replay_keys(expires_at);
CREATE INDEX idx_webhook_replay_delivery ON scheduled_task_webhook_replay_keys(delivery_id);
CREATE INDEX idx_webhook_pending_due ON scheduled_task_webhook_pending(state, due_at);
CREATE UNIQUE INDEX idx_webhook_pending_key
    ON scheduled_task_webhook_pending(task_id, coalesce_key) WHERE state = 'pending';
CREATE INDEX orchestration_v2_projection_turn_items_stop_request_idx
    ON orchestration_v2_projection_turn_items(thread_id, run_id)
    WHERE type = 'run_interrupt_request';
CREATE INDEX orchestration_v2_projection_turn_items_waiting_link_idx
    ON orchestration_v2_projection_turn_items(thread_id, updated_at DESC, turn_item_id DESC)
    WHERE type = 'link_request' AND status = 'waiting';
CREATE INDEX orchestration_v2_projection_turn_items_active_ordinal_idx
    ON orchestration_v2_projection_turn_items(thread_id, ordinal, turn_item_id)
    WHERE status IN ('pending', 'running', 'waiting');
CREATE INDEX orchestration_v2_projection_turn_items_active_run_idx
    ON orchestration_v2_projection_turn_items(thread_id, run_id)
    WHERE status IN ('pending', 'running', 'waiting');
CREATE INDEX orchestration_v2_projection_subagents_settled_idx
    ON orchestration_v2_projection_subagents(thread_id, status, completion_delivery_state, run_id);
CREATE INDEX orchestration_v2_projection_threads_subagent_result_idx
    ON orchestration_v2_projection_threads(
      thread_id, json_extract(payload_json, '$.lineage.parentThreadId')
    )
    WHERE CASE WHEN json_valid(payload_json) THEN
      json_extract(payload_json, '$.lineage.relationshipToParent') = 'subagent'
      AND json_extract(payload_json, '$.lineage.parentThreadId') IS NOT NULL
      AND json_extract(payload_json, '$.forkedFrom.type') = 'node'
      ELSE 0 END;
CREATE INDEX orchestration_v2_projection_runs_delegated_delivery_idx
    ON orchestration_v2_projection_runs(thread_id)
    WHERE CASE WHEN json_valid(payload_json)
      THEN json_type(payload_json, '$.delegatedCompletion.delivery') = 'object'
      ELSE 0 END;
CREATE INDEX orchestration_v2_projection_turn_items_conversation_idx
    ON orchestration_v2_projection_turn_items(thread_id, ordinal, type, run_id)
    WHERE type NOT IN (
      'command_execution', 'reasoning', 'file_change', 'file_search', 'web_search', 'dynamic_tool'
    );
CREATE INDEX orchestration_v2_projection_nodes_settled_leaf_idx
    ON orchestration_v2_projection_nodes(
      thread_id,
      (kind IN ('tool_call', 'assistant_message', 'reasoning')
        AND status IN ('completed', 'interrupted', 'failed', 'cancelled', 'rolled_back')
        AND run_id IS NOT NULL)
    );
CREATE INDEX orchestration_v2_projection_context_transfers_source_thread_status_idx
    ON orchestration_v2_projection_context_transfers(source_thread_id, status);
CREATE INDEX orchestration_v2_projection_runs_last_executed_idx
    ON orchestration_v2_projection_runs(
      thread_id, (completed_at IS NULL) DESC, completed_at DESC, ordinal DESC, run_id DESC
    )
    WHERE status <> 'queued'
      AND NOT (status = 'cancelled' AND json_extract(payload_json, '$.startedAt') IS NULL);
CREATE INDEX orchestration_v2_projection_turn_items_thread_type_idx
    ON orchestration_v2_projection_turn_items(thread_id, type, status);
CREATE INDEX orchestration_v2_projection_threads_worktree_path_idx
    ON orchestration_v2_projection_threads(
      CASE WHEN json_valid(payload_json) THEN json_extract(payload_json, '$.worktreePath') END
    );
CREATE INDEX orchestration_v2_projection_threads_root_thread_idx
    ON orchestration_v2_projection_threads(
      CASE WHEN json_valid(payload_json) THEN json_extract(payload_json, '$.lineage.rootThreadId') END
    );
CREATE INDEX orchestration_v2_projection_runs_run_status_idx
    ON orchestration_v2_projection_runs(run_id, status);
CREATE INDEX orchestration_v2_projection_turn_items_queued_input_idx
    ON orchestration_v2_projection_turn_items(run_id)
    WHERE type = 'user_message'
      AND json_valid(payload_json)
      AND json_extract(payload_json, '$.inputIntent') = 'queued_turn';
CREATE INDEX orchestration_v2_projection_provider_turns_attempt_idx
    ON orchestration_v2_projection_provider_turns(run_attempt_id);
CREATE INDEX orchestration_v2_projection_turn_items_message_ordinal_idx
    ON orchestration_v2_projection_turn_items(
      thread_id, json_extract(payload_json, '$.messageId'), ordinal
    )
    WHERE type IN ('user_message', 'assistant_message') AND json_valid(payload_json);
CREATE TRIGGER orchestration_v2_message_search_insert
      AFTER INSERT ON orchestration_v2_projection_messages
      WHEN 
  
  new.streaming = 0
  AND (
    new.role = 'assistant'
    OR (
      new.role = 'user'
      AND COALESCE(json_extract(new.payload_json, '$.createdBy'), 'user') = 'user'
    )
  )
  AND NOT EXISTS (
    SELECT 1 FROM orchestration_v2_projection_threads AS subagent_threads
    WHERE subagent_threads.thread_id = new.thread_id
      AND json_extract(subagent_threads.payload_json, '$.lineage.relationshipToParent') = 'subagent'
  )

  AND 
  CASE
    WHEN substr(json_extract(new.payload_json, '$.text'), 1, 11) = 'Ultrathink:' COLLATE NOCASE
      THEN ltrim(substr(json_extract(new.payload_json, '$.text'), 12), '	
                  　﻿')
    ELSE json_extract(new.payload_json, '$.text')
  END COLLATE BINARY <> '[User attached one or more files without additional text. Respond using the conversation context and the attached files.]'


      BEGIN
        INSERT INTO orchestration_v2_message_search (rowid, text)
        SELECT new.rowid, replace(replace(COALESCE(new.search_text, json_extract(new.payload_json, '$.text')), '’', ''''), '‘', '''')
        WHERE json_extract(new.payload_json, '$.text') IS NOT NULL;
      END;
CREATE TRIGGER orchestration_v2_message_search_update
      AFTER UPDATE ON orchestration_v2_projection_messages
      WHEN (old.streaming = 0 OR new.streaming = 0)
        AND (
          old.streaming IS NOT new.streaming
          OR old.role IS NOT new.role
          OR replace(replace(COALESCE(old.search_text, json_extract(old.payload_json, '$.text')), '’', ''''), '‘', '''') IS NOT replace(replace(COALESCE(new.search_text, json_extract(new.payload_json, '$.text')), '’', ''''), '‘', '''')
          OR json_extract(old.payload_json, '$.createdBy')
            IS NOT json_extract(new.payload_json, '$.createdBy')
        )
      BEGIN
        DELETE FROM orchestration_v2_message_search WHERE rowid = old.rowid AND old.streaming = 0;
        INSERT INTO orchestration_v2_message_search (rowid, text)
        SELECT new.rowid, replace(replace(COALESCE(new.search_text, json_extract(new.payload_json, '$.text')), '’', ''''), '‘', '''')
        WHERE 
  
  new.streaming = 0
  AND (
    new.role = 'assistant'
    OR (
      new.role = 'user'
      AND COALESCE(json_extract(new.payload_json, '$.createdBy'), 'user') = 'user'
    )
  )
  AND NOT EXISTS (
    SELECT 1 FROM orchestration_v2_projection_threads AS subagent_threads
    WHERE subagent_threads.thread_id = new.thread_id
      AND json_extract(subagent_threads.payload_json, '$.lineage.relationshipToParent') = 'subagent'
  )

  AND 
  CASE
    WHEN substr(json_extract(new.payload_json, '$.text'), 1, 11) = 'Ultrathink:' COLLATE NOCASE
      THEN ltrim(substr(json_extract(new.payload_json, '$.text'), 12), '	
                  　﻿')
    ELSE json_extract(new.payload_json, '$.text')
  END COLLATE BINARY <> '[User attached one or more files without additional text. Respond using the conversation context and the attached files.]'


          AND json_extract(new.payload_json, '$.text') IS NOT NULL;
      END;
CREATE TRIGGER orchestration_v2_message_search_delete
      AFTER DELETE ON orchestration_v2_projection_messages
      WHEN old.streaming = 0
      BEGIN
        DELETE FROM orchestration_v2_message_search WHERE rowid = old.rowid;
      END;
CREATE TRIGGER orchestration_v2_turn_item_counts_insert
    AFTER INSERT ON orchestration_v2_projection_turn_items
    BEGIN
      INSERT INTO orchestration_v2_projection_turn_item_counts (thread_id, run_key, item_count)
      VALUES (new.thread_id, COALESCE(new.run_id, ''), 1)
      ON CONFLICT (thread_id, run_key) DO UPDATE SET item_count = item_count + 1;
    END;
CREATE TRIGGER orchestration_v2_turn_item_counts_update
    AFTER UPDATE OF thread_id, run_id ON orchestration_v2_projection_turn_items
    WHEN old.thread_id IS NOT new.thread_id OR old.run_id IS NOT new.run_id
    BEGIN
      UPDATE orchestration_v2_projection_turn_item_counts SET item_count = item_count - 1
      WHERE thread_id = old.thread_id AND run_key = COALESCE(old.run_id, '');
      DELETE FROM orchestration_v2_projection_turn_item_counts
      WHERE thread_id = old.thread_id AND run_key = COALESCE(old.run_id, '') AND item_count <= 0;
      INSERT INTO orchestration_v2_projection_turn_item_counts (thread_id, run_key, item_count)
      VALUES (new.thread_id, COALESCE(new.run_id, ''), 1)
      ON CONFLICT (thread_id, run_key) DO UPDATE SET item_count = item_count + 1;
    END;
CREATE TRIGGER orchestration_v2_turn_item_counts_delete
    AFTER DELETE ON orchestration_v2_projection_turn_items
    BEGIN
      UPDATE orchestration_v2_projection_turn_item_counts SET item_count = item_count - 1
      WHERE thread_id = old.thread_id AND run_key = COALESCE(old.run_id, '');
      DELETE FROM orchestration_v2_projection_turn_item_counts
      WHERE thread_id = old.thread_id AND run_key = COALESCE(old.run_id, '') AND item_count <= 0;
    END;
