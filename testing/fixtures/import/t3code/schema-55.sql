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
    , default_model_selection_json TEXT, default_thread_env_mode TEXT, favicon_path TEXT, auto_pull INTEGER NOT NULL DEFAULT 0, project_icon_json TEXT);
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
    , client_surface TEXT, client_app_version TEXT);
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
    );
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
    );
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
    , driver TEXT, provider_instance_id TEXT);
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
    );
CREATE TABLE orchestration_v2_legacy_imports (
      thread_id TEXT PRIMARY KEY,
      source_updated_at TEXT NOT NULL,
      shell_imported_at TEXT NOT NULL,
      transcript_imported_at TEXT,
      imported_message_count INTEGER NOT NULL DEFAULT 0,
      last_error TEXT
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
CREATE INDEX idx_projection_turns_thread_keyset
    ON projection_turns(thread_id, requested_at, turn_id);
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
CREATE INDEX orchestration_v2_projection_context_transfers_source_thread_idx ON orchestration_v2_projection_context_transfers(source_thread_id);
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
