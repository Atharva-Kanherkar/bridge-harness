CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
CREATE TABLE attachments (
                    id TEXT PRIMARY KEY,
                    type TEXT,
                    original_name TEXT,
                    path TEXT,
                    is_loading INTEGER DEFAULT 0,
                    session_id TEXT,
                    session_message_id TEXT,
                    is_draft INTEGER DEFAULT 1,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                , comment_id TEXT);
CREATE TABLE env_vars (
                    key TEXT,
                    scope TEXT,
                    context TEXT,
                    value TEXT
                , version INTEGER DEFAULT 1, created_at TEXT, updated_at TEXT, id TEXT, user_id TEXT);
CREATE TABLE favorite_models (
                    organization_id TEXT,
                    user_id TEXT,
                    id TEXT,
                    position INTEGER,
                    configuration TEXT,
                    created_at TEXT,
                    updated_at TEXT,
                    version INTEGER DEFAULT 1,
                    PRIMARY KEY (organization_id, user_id, id)
                );
CREATE TABLE identity_associations (
                    local_id TEXT,
                    server_id TEXT
                );
CREATE TABLE local_workspace_deletion_tombstones (
                    workspace_id TEXT,
                    revision INTEGER,
                    deleted_at TEXT
                );
CREATE TABLE local_workspace_number_counters (
                    organization_id TEXT PRIMARY KEY,
                    last_workspace_number INTEGER DEFAULT 0,
                    updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );
CREATE TABLE local_workspace_revision_counter (
                    id INTEGER PRIMARY KEY,
                    latest_revision INTEGER
                );
CREATE TABLE migration_rollbacks (
                    version INTEGER PRIMARY KEY,
                    down_sql TEXT NOT NULL
                );
CREATE TABLE navigation_section_memberships (
                    id TEXT PRIMARY KEY,
                    section_id TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    created_at TEXT NOT NULL
                , version INTEGER DEFAULT 1, updated_at TEXT, position INTEGER);
CREATE TABLE navigation_sections (
                    id TEXT PRIMARY KEY,
                    organization_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    icon_name TEXT,
                    sort_order INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                , emoji TEXT, kind TEXT, version INTEGER DEFAULT 1, snapshot_changed_at TEXT);
CREATE TABLE "repo_settings" (
                    repo_id TEXT,
                    key TEXT,
                    value TEXT,
                    created_at TEXT DEFAULT (datetime('now')),
                    updated_at TEXT DEFAULT (datetime('now')),
                    version INTEGER DEFAULT 1,
                    PRIMARY KEY (repo_id, key)
                );
CREATE TABLE repos (
                    id TEXT PRIMARY KEY,
                    remote_url TEXT,
                    name TEXT,
                    default_branch TEXT DEFAULT 'main',
                    root_path TEXT,
                    setup_script TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                , storage_version INTEGER DEFAULT 1, archive_script TEXT, display_order INTEGER DEFAULT 0, run_script TEXT, run_script_mode TEXT DEFAULT 'concurrent', remote TEXT, custom_prompt_code_review TEXT, custom_prompt_create_pr TEXT, custom_prompt_rename_branch TEXT, conductor_config TEXT, custom_prompt_general TEXT, icon TEXT, hidden INTEGER DEFAULT 0, custom_prompt_fix_errors TEXT, custom_prompt_resolve_merge_conflicts TEXT, file_include_globs TEXT, spotlight_testing INTEGER DEFAULT 0, version INTEGER DEFAULT 1, lifecycle_step TEXT, failure_code TEXT, failure_reason TEXT, failure_phase TEXT, root_missing INTEGER);
CREATE TABLE review_comments (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT,
                    version INTEGER DEFAULT 1,
                    created_at TEXT,
                    updated_at TEXT,
                    review_ready_at TEXT,
                    remote_updated_at TEXT,
                    edited_at TEXT,
                    file_path TEXT,
                    line_number INTEGER,
                    end_line_number INTEGER,
                    body TEXT,
                    state TEXT,
                    location TEXT,
                    remote_url TEXT,
                    author TEXT,
                    author_avatar_url TEXT,
                    thread_id TEXT,
                    reply_to_comment_id TEXT,
                    suggestion_base_code TEXT,
                    is_outdated INTEGER,
                    is_resolved INTEGER
                );
CREATE TABLE session_assets_outbox (
                    attachment_id TEXT PRIMARY KEY,
                    message_id TEXT,
                    source_ref TEXT,
                    original_name TEXT,
                    state TEXT,
                    upload_attempt_count INTEGER DEFAULT 0,
                    next_upload_attempt_at TEXT,
                    last_error TEXT,
                    created_at TEXT,
                    updated_at TEXT
                , asset_id TEXT, relative_path TEXT);
CREATE TABLE session_events (
                    session_id TEXT,
                    session_index INTEGER,
                    event_id TEXT,
                    event_type TEXT,
                    content TEXT,
                    received_at TEXT
                , reverted_at TEXT);
CREATE TABLE session_messages (
                    id TEXT PRIMARY KEY,
                    session_id TEXT,
                    role TEXT,
                    content TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                , sent_at TEXT, full_message TEXT, cancelled_at TEXT, model TEXT, sdk_message_id TEXT, last_assistant_message_id TEXT, turn_id TEXT, is_resumable_message INTEGER, queue_order INTEGER, sender_id TEXT, sender_session_id TEXT, sender_api_key_name TEXT, sender_source TEXT, sender_turn_id TEXT, sender_message_id TEXT);
CREATE TABLE session_messages_outbox (
                    message_id TEXT PRIMARY KEY,
                    session_id TEXT,
                    delivery_payload TEXT,
                    config TEXT,
                    mode TEXT,
                    asset_ids TEXT,
                    queue_order INTEGER,
                    state TEXT,
                    send_attempt_count INTEGER DEFAULT 0,
                    next_send_attempt_at TEXT,
                    last_error TEXT,
                    created_at TEXT,
                    updated_at TEXT
                );
CREATE TABLE session_messages_outbox_state (
                    session_id TEXT,
                    paused_at TEXT,
                    updated_at TEXT
                );
CREATE TABLE session_state (
                    session_id TEXT PRIMARY KEY,
                    live_state TEXT,
                    live_state_updated_at TEXT,
                    config TEXT,
                    config_updated_at TEXT
                );
CREATE TABLE "sessions" (
                        id TEXT PRIMARY KEY,
                        status TEXT DEFAULT 'idle',
                        claude_session_id TEXT,
                        unread_count INTEGER DEFAULT 0,
                        freshly_compacted INTEGER DEFAULT 0,
                        context_token_count INTEGER DEFAULT 0,
                        created_at TEXT NOT NULL DEFAULT (datetime('now')),
                        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                    , is_compacting INTEGER DEFAULT 0, model TEXT, permission_mode TEXT DEFAULT 'default', DEPRECATED_thinking_level TEXT DEFAULT 'NONE', last_user_message_at TEXT, resume_session_at TEXT, workspace_id TEXT, is_hidden INTEGER DEFAULT 0, agent_type, title TEXT DEFAULT 'Untitled', context_used_percent FLOAT, DEPRECATED_thinking_enabled INTEGER DEFAULT 1, codex_thinking_level TEXT, fast_mode INTEGER DEFAULT 0, agent_personality TEXT, claude_effort_level TEXT, feed_offset INTEGER, queue_paused_at TEXT, remote_archived_at TEXT, acp_config TEXT, last_message_at TEXT, created_by_session_id TEXT, created_by_turn_id TEXT, closed_at TEXT, created_by_message_id TEXT, estimated_cost_usd REAL, total_tokens INTEGER, version INTEGER DEFAULT 1);
CREATE TABLE settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                , deprecated_at TEXT, version INTEGER DEFAULT 1, user_id TEXT, value_changed_at TEXT);
CREATE TABLE symlinks_pending_deletion (
                    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
                    path TEXT,
                    version INTEGER DEFAULT 1,
                    created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    organization_id TEXT,
                    workspace_id TEXT,
                    target_path TEXT
                );
CREATE TABLE terminal_sessions (
                    session_id TEXT PRIMARY KEY,
                    workspace_id TEXT,
                    started_at TEXT,
                    ended_at TEXT,
                    cwd TEXT,
                    cols INTEGER,
                    rows INTEGER,
                    rehydrate_sequences TEXT,
                    alternate_screen INTEGER,
                    bracketed_paste INTEGER
                , has_received_user_input INTEGER, version INTEGER DEFAULT 1, created_at TEXT, updated_at TEXT);
CREATE TABLE workspace_links (
                    id TEXT PRIMARY KEY,
                    version INTEGER DEFAULT 1,
                    created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    organization_id TEXT,
                    source_workspace_id TEXT,
                    kind TEXT,
                    target_workspace_id TEXT,
                    directory_path TEXT,
                    position INTEGER
                );
CREATE TABLE workspaces (
                    local_id TEXT PRIMARY KEY,
                    repository_id TEXT,
                    DEPRECATED_city_name TEXT,
                    directory_name TEXT,
                    DEPRECATED_archived INTEGER DEFAULT 0,
                    active_session_id TEXT,
                    branch TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                , unread INTEGER DEFAULT 0, placeholder_branch_name TEXT, state TEXT DEFAULT 'active', initialization_parent_branch TEXT, big_terminal_mode INTEGER DEFAULT 0, setup_log_path TEXT, initialization_log_path TEXT, initialization_files_copied INTEGER, pinned_at TEXT, linked_workspace_ids TEXT, notes TEXT, intended_target_branch TEXT, manual_status TEXT, derived_status TEXT DEFAULT 'in-progress', archive_commit TEXT, pr_title TEXT, pr_description TEXT, secondary_directory_name TEXT, linked_directory_paths TEXT, hosting_server_url TEXT, sandbox_provider TEXT, workspace_path TEXT, user_set_workspace_name INTEGER DEFAULT 0, user_set_branch_name INTEGER DEFAULT 0, workspace_name TEXT, permission_level TEXT, creator_user_id TEXT, remote_file_sync_enabled INTEGER DEFAULT 0, creator_client_id TEXT, organization_id TEXT, assignee_user_id TEXT, watcher_user_ids TEXT, workspace_template_id TEXT, is_computer_admin INTEGER NOT NULL DEFAULT 0, auto_forward_enabled INTEGER DEFAULT 1, restricted INTEGER, access_level TEXT, number INTEGER, port_forwarding_enabled INTEGER DEFAULT 0, id TEXT, revision INTEGER, last_activity_at TEXT, created_by_session_id TEXT, created_by_turn_id TEXT, initialization_error TEXT, version INTEGER DEFAULT 1, created_by_message_id TEXT, lifecycle_step TEXT, failure_reason TEXT, failure_phase TEXT, setup_script_failure_reason TEXT, failure_code TEXT, lifecycle_warning TEXT, branch_detached INTEGER);
CREATE INDEX idx_attachments_is_draft ON attachments(is_draft);
CREATE INDEX idx_attachments_session_id ON attachments(session_id);
CREATE INDEX idx_attachments_session_message_id ON attachments(session_message_id);
CREATE UNIQUE INDEX idx_env_vars_id ON env_vars(id);
CREATE UNIQUE INDEX idx_env_vars_key_scope_context
                    ON env_vars(key, scope, context);
CREATE INDEX idx_favorite_models_scope_position
                    ON favorite_models (organization_id, user_id, position);
CREATE UNIQUE INDEX idx_identity_associations_local_id
                    ON identity_associations (local_id);
CREATE UNIQUE INDEX idx_identity_associations_server_id
                    ON identity_associations (server_id);
CREATE UNIQUE INDEX idx_local_workspace_deletion_tombstones_workspace_id
                    ON local_workspace_deletion_tombstones (workspace_id);
CREATE INDEX idx_navigation_section_memberships_section_position
                    ON navigation_section_memberships (section_id, position);
CREATE INDEX idx_navigation_sections_scope_sort_order
                    ON navigation_sections (organization_id, user_id, sort_order);
CREATE UNIQUE INDEX idx_repo_settings_repo_id_key
                    ON repo_settings (repo_id, key);
CREATE INDEX idx_review_comments_workspace ON review_comments (workspace_id);
CREATE INDEX idx_review_comments_workspace_thread ON review_comments (workspace_id, thread_id);
CREATE INDEX idx_review_comments_workspace_timeline ON review_comments (workspace_id, COALESCE(review_ready_at, created_at), id);
CREATE INDEX idx_session_assets_outbox_due
                    ON session_assets_outbox(state, next_upload_attempt_at);
CREATE INDEX idx_session_assets_outbox_message
                    ON session_assets_outbox(message_id, created_at);
CREATE UNIQUE INDEX idx_session_events_event_id
                    ON session_events (session_id, event_id);
CREATE UNIQUE INDEX idx_session_events_session_index
                    ON session_events (session_id, session_index);
CREATE INDEX idx_session_messages_cancelled_at ON session_messages(session_id, cancelled_at);
CREATE INDEX idx_session_messages_outbox_due
                    ON session_messages_outbox(state, next_send_attempt_at);
CREATE INDEX idx_session_messages_outbox_order
                    ON session_messages_outbox(session_id, queue_order, created_at);
CREATE INDEX idx_session_messages_outbox_session
                    ON session_messages_outbox(session_id, created_at);
CREATE UNIQUE INDEX idx_session_messages_outbox_state_session
                    ON session_messages_outbox_state(session_id);
CREATE INDEX idx_session_messages_queued_session_id
                ON session_messages(session_id)
                WHERE sent_at IS NULL AND cancelled_at IS NULL;
CREATE INDEX idx_session_messages_sdk_message_id
                ON session_messages(session_id, sdk_message_id)
                WHERE sdk_message_id IS NOT NULL;
CREATE INDEX idx_session_messages_sent_at ON session_messages(session_id, sent_at);
CREATE INDEX idx_session_messages_turn_id
                ON session_messages(turn_id);
CREATE INDEX idx_session_messages_user_turns
                ON session_messages(session_id, role, turn_id, id, sent_at, cancelled_at)
                WHERE role = 'user';
CREATE INDEX idx_sessions_created_by_session_id
                    ON sessions (created_by_session_id);
CREATE INDEX idx_sessions_workspace_id ON sessions(workspace_id);
CREATE INDEX idx_settings_user_id ON settings (user_id);
CREATE INDEX idx_symlinks_pending_deletion_path
                    ON symlinks_pending_deletion (path);
CREATE INDEX idx_symlinks_pending_deletion_scope_path
                    ON symlinks_pending_deletion (organization_id, path);
CREATE INDEX idx_workspace_links_organization_target
                    ON workspace_links (organization_id, target_workspace_id, source_workspace_id);
CREATE INDEX idx_workspace_links_source_kind_position
                    ON workspace_links (source_workspace_id, kind, position, id);
CREATE UNIQUE INDEX idx_workspaces_local_organization_number
                    ON workspaces (organization_id, number)
                    WHERE hosting_server_url IS NULL OR hosting_server_url IN ('', '/');
CREATE INDEX idx_workspaces_organization_id ON workspaces(organization_id);
CREATE INDEX idx_workspaces_repository_id ON workspaces(repository_id);
CREATE TRIGGER initialize_env_vars_updated_at AFTER INSERT ON env_vars
                WHEN NEW.updated_at IS NULL
                BEGIN
                    UPDATE env_vars SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER initialize_favorite_models_updated_at AFTER INSERT ON favorite_models
                WHEN NEW.updated_at IS NULL
                BEGIN
                    UPDATE favorite_models SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER initialize_navigation_section_memberships_updated_at AFTER INSERT ON navigation_section_memberships
                WHEN NEW.updated_at IS NULL
                BEGIN
                    UPDATE navigation_section_memberships SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER initialize_terminal_sessions_updated_at AFTER INSERT ON terminal_sessions
                WHEN NEW.updated_at IS NULL
                BEGIN
                    UPDATE terminal_sessions SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER local_workspace_revision_after_delete
                AFTER DELETE ON workspaces
                WHEN OLD.hosting_server_url IS NULL
                    OR OLD.hosting_server_url IN ('', '/')
                BEGIN
                    UPDATE local_workspace_revision_counter
                    SET latest_revision = latest_revision + 1
                    WHERE id = 1;
                    INSERT OR REPLACE INTO local_workspace_deletion_tombstones
                        (workspace_id, revision, deleted_at)
                    SELECT
                        OLD.local_id,
                        latest_revision,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    FROM local_workspace_revision_counter
                    WHERE id = 1;
                END;
CREATE TRIGGER local_workspace_revision_after_insert
                AFTER INSERT ON workspaces
                WHEN NEW.hosting_server_url IS NULL
                    OR NEW.hosting_server_url IN ('', '/')
                BEGIN
                    UPDATE local_workspace_revision_counter
                    SET latest_revision = latest_revision + 1
                    WHERE id = 1;
                    UPDATE workspaces
                    SET revision = (
                        SELECT latest_revision
                        FROM local_workspace_revision_counter
                        WHERE id = 1
                    )
                    WHERE local_id = NEW.local_id;
                    DELETE FROM local_workspace_deletion_tombstones
                    WHERE workspace_id = NEW.local_id;
                END;
CREATE TRIGGER local_workspace_revision_after_update
                AFTER UPDATE ON workspaces
                WHEN (
                    NEW.hosting_server_url IS NULL
                    OR NEW.hosting_server_url IN ('', '/')
                ) AND (
                    OLD.local_id IS NOT NEW.local_id
                    OR OLD.id IS NOT NEW.id
                    OR OLD.repository_id IS NOT NEW.repository_id
                    OR OLD.directory_name IS NOT NEW.directory_name
                    OR OLD.secondary_directory_name IS NOT NEW.secondary_directory_name
                    OR OLD.workspace_name IS NOT NEW.workspace_name
                    OR OLD.workspace_path IS NOT NEW.workspace_path
                    OR OLD.state IS NOT NEW.state
                    OR OLD.branch IS NOT NEW.branch
                    OR OLD.intended_target_branch IS NOT NEW.intended_target_branch
                    OR OLD.manual_status IS NOT NEW.manual_status
                    OR OLD.derived_status IS NOT NEW.derived_status
                    OR OLD.hosting_server_url IS NOT NEW.hosting_server_url
                    OR OLD.sandbox_provider IS NOT NEW.sandbox_provider
                    OR OLD.number IS NOT NEW.number
                    OR OLD.organization_id IS NOT NEW.organization_id
                    OR OLD.creator_user_id IS NOT NEW.creator_user_id
                    OR OLD.creator_client_id IS NOT NEW.creator_client_id
                    OR OLD.assignee_user_id IS NOT NEW.assignee_user_id
                    OR OLD.watcher_user_ids IS NOT NEW.watcher_user_ids
                    OR OLD.permission_level IS NOT NEW.permission_level
                    OR OLD.restricted IS NOT NEW.restricted
                    OR OLD.access_level IS NOT NEW.access_level
                    OR OLD.workspace_template_id IS NOT NEW.workspace_template_id
                    OR OLD.is_computer_admin IS NOT NEW.is_computer_admin
                    OR OLD.created_at IS NOT NEW.created_at
                    OR OLD.last_activity_at IS NOT NEW.last_activity_at
                )
                BEGIN
                    UPDATE local_workspace_revision_counter
                    SET latest_revision = latest_revision + 1
                    WHERE id = 1;
                    UPDATE workspaces
                    SET revision = (
                        SELECT latest_revision
                        FROM local_workspace_revision_counter
                        WHERE id = 1
                    )
                    WHERE local_id = NEW.local_id;
                END;
CREATE TRIGGER update_env_vars_updated_at AFTER UPDATE ON env_vars
                BEGIN
                    UPDATE env_vars SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_favorite_models_updated_at AFTER UPDATE ON favorite_models
                BEGIN
                    UPDATE favorite_models SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_local_workspace_number_counters_updated_at
                AFTER UPDATE ON local_workspace_number_counters
                BEGIN
                    UPDATE local_workspace_number_counters
                    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_navigation_section_memberships_updated_at AFTER UPDATE ON navigation_section_memberships
                BEGIN
                    UPDATE navigation_section_memberships SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_navigation_sections_updated_at AFTER UPDATE ON navigation_sections
                BEGIN
                    UPDATE navigation_sections SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_repo_settings_updated_at AFTER UPDATE ON repo_settings
                BEGIN
                    UPDATE repo_settings SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_repos_updated_at AFTER UPDATE ON repos
                BEGIN
                    UPDATE repos SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_review_comments_updated_at AFTER UPDATE ON review_comments
                BEGIN
                    UPDATE review_comments SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_session_assets_outbox_updated_at AFTER UPDATE ON session_assets_outbox
                BEGIN
                    UPDATE session_assets_outbox SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_session_messages_outbox_state_updated_at AFTER UPDATE ON session_messages_outbox_state
                BEGIN
                    UPDATE session_messages_outbox_state SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_session_messages_outbox_updated_at AFTER UPDATE ON session_messages_outbox
                BEGIN
                    UPDATE session_messages_outbox SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_sessions_updated_at AFTER UPDATE ON sessions
                BEGIN
                    UPDATE sessions SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_settings_updated_at AFTER UPDATE ON settings
                BEGIN
                    UPDATE settings SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_symlinks_pending_deletion_updated_at AFTER UPDATE ON symlinks_pending_deletion
                BEGIN
                    UPDATE symlinks_pending_deletion SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_terminal_sessions_updated_at AFTER UPDATE ON terminal_sessions
                BEGIN
                    UPDATE terminal_sessions SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_workspace_links_updated_at AFTER UPDATE ON workspace_links
                BEGIN
                    UPDATE workspace_links SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
CREATE TRIGGER update_workspaces_updated_at AFTER UPDATE ON workspaces
                BEGIN
                    UPDATE workspaces SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE rowid = NEW.rowid;
                END;
