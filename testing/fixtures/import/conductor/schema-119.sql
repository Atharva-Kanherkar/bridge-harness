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
CREATE TABLE diff_comments (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT,
                    file_path TEXT,
                    line_number INTEGER,
                    body TEXT,
                    state TEXT,
                    location TEXT,
                    created_at INTEGER NOT NULL,
                    remote_url TEXT,
                    author TEXT,
                    thread_id TEXT,
                    reply_to_comment_id TEXT,
                    DEPRECATED_update_memory INTEGER
                , updated_at INTEGER, author_avatar_url TEXT, is_outdated INTEGER, is_resolved INTEGER, end_line_number INTEGER);
CREATE TABLE env_vars (
                    key TEXT,
                    scope TEXT,
                    context TEXT,
                    value TEXT
                );
CREATE TABLE migration_rollbacks (
                    version INTEGER PRIMARY KEY,
                    down_sql TEXT NOT NULL
                );
CREATE TABLE port_forwards (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT,
                    remote_port INTEGER,
                    local_port INTEGER,
                    protocol TEXT DEFAULT 'tcp',
                    label TEXT,
                    enabled INTEGER DEFAULT 1,
                    created_at TEXT DEFAULT (datetime('now'))
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
                , storage_version INTEGER DEFAULT 1, archive_script TEXT, display_order INTEGER DEFAULT 0, run_script TEXT, run_script_mode TEXT DEFAULT 'concurrent', remote TEXT, custom_prompt_code_review TEXT, custom_prompt_create_pr TEXT, custom_prompt_rename_branch TEXT, conductor_config TEXT, custom_prompt_general TEXT, icon TEXT, hidden INTEGER DEFAULT 0, custom_prompt_fix_errors TEXT, custom_prompt_resolve_merge_conflicts TEXT, file_include_globs TEXT, spotlight_testing INTEGER DEFAULT 0);
CREATE TABLE session_messages (
                    id TEXT PRIMARY KEY,
                    session_id TEXT,
                    role TEXT,
                    content TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                , sent_at TEXT, full_message TEXT, cancelled_at TEXT, model TEXT, sdk_message_id TEXT, last_assistant_message_id TEXT, turn_id TEXT, is_resumable_message INTEGER, queue_order INTEGER, sender_id TEXT, sender_session_id TEXT, sender_api_key_name TEXT);
CREATE TABLE "sessions" (
                        id TEXT PRIMARY KEY,
                        status TEXT DEFAULT 'idle',
                        claude_session_id TEXT,
                        unread_count INTEGER DEFAULT 0,
                        freshly_compacted INTEGER DEFAULT 0,
                        context_token_count INTEGER DEFAULT 0,
                        created_at TEXT NOT NULL DEFAULT (datetime('now')),
                        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                    , is_compacting INTEGER DEFAULT 0, model TEXT, permission_mode TEXT DEFAULT 'default', DEPRECATED_thinking_level TEXT DEFAULT 'NONE', last_user_message_at TEXT, resume_session_at TEXT, workspace_id TEXT, is_hidden INTEGER DEFAULT 0, agent_type, title TEXT DEFAULT 'Untitled', context_used_percent FLOAT, DEPRECATED_thinking_enabled INTEGER DEFAULT 1, codex_thinking_level TEXT, fast_mode INTEGER DEFAULT 0, agent_personality TEXT, claude_effort_level TEXT, feed_offset INTEGER, queue_paused_at TEXT);
CREATE TABLE settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                , deprecated_at TEXT);
CREATE TABLE symlinks_pending_deletion (
                    path TEXT
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
                , has_received_user_input INTEGER);
CREATE TABLE workspaces (
                    id TEXT PRIMARY KEY,
                    repository_id TEXT,
                    DEPRECATED_city_name TEXT,
                    directory_name TEXT,
                    DEPRECATED_archived INTEGER DEFAULT 0,
                    active_session_id TEXT,
                    branch TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                , unread INTEGER DEFAULT 0, placeholder_branch_name TEXT, state TEXT DEFAULT 'active', initialization_parent_branch TEXT, big_terminal_mode INTEGER DEFAULT 0, setup_log_path TEXT, initialization_log_path TEXT, initialization_files_copied INTEGER, pinned_at TEXT, linked_workspace_ids TEXT, notes TEXT, intended_target_branch TEXT, manual_status TEXT, derived_status TEXT DEFAULT 'in-progress', archive_commit TEXT, pr_title TEXT, pr_description TEXT, secondary_directory_name TEXT, linked_directory_paths TEXT, hosting_server_url TEXT, sandbox_provider TEXT, workspace_path TEXT, user_set_workspace_name INTEGER DEFAULT 0, user_set_branch_name INTEGER DEFAULT 0, workspace_name TEXT, permission_level TEXT, creator_user_id TEXT, remote_file_sync_enabled INTEGER DEFAULT 0, creator_client_id TEXT, organization_id TEXT, assignee_user_id TEXT, watcher_user_ids TEXT, workspace_template_id TEXT, is_computer_admin INTEGER NOT NULL DEFAULT 0);
CREATE INDEX idx_attachments_is_draft ON attachments(is_draft);
CREATE INDEX idx_attachments_session_id ON attachments(session_id);
CREATE INDEX idx_attachments_session_message_id ON attachments(session_message_id);
CREATE INDEX idx_diff_comments_workspace ON diff_comments(workspace_id);
CREATE UNIQUE INDEX idx_env_vars_key_scope_context
                    ON env_vars(key, scope, context);
CREATE UNIQUE INDEX idx_port_forwards_local_port
                    ON port_forwards(local_port);
CREATE UNIQUE INDEX idx_port_forwards_workspace_remote
                    ON port_forwards(workspace_id, remote_port);
CREATE INDEX idx_session_messages_cancelled_at ON session_messages(session_id, cancelled_at);
CREATE INDEX idx_session_messages_queued_session_id
                ON session_messages(session_id)
                WHERE sent_at IS NULL AND cancelled_at IS NULL;
CREATE INDEX idx_session_messages_sent_at ON session_messages(session_id, sent_at);
CREATE INDEX idx_session_messages_turn_id
                ON session_messages(turn_id);
CREATE INDEX idx_sessions_workspace_id ON sessions(workspace_id);
CREATE INDEX idx_workspaces_organization_id ON workspaces(organization_id);
CREATE INDEX idx_workspaces_repository_id ON workspaces(repository_id);
CREATE TRIGGER update_repos_updated_at
                AFTER UPDATE ON repos
                BEGIN
                UPDATE repos SET updated_at = datetime('now')
                WHERE id = NEW.id;
                END;
CREATE TRIGGER update_sessions_updated_at
                AFTER UPDATE ON sessions
                BEGIN
                UPDATE sessions SET updated_at = datetime('now')
                WHERE id = NEW.id;
                END;
CREATE TRIGGER update_settings_updated_at
                AFTER UPDATE ON settings
                BEGIN
                UPDATE settings SET updated_at = datetime('now')
                WHERE key = NEW.key;
                END;
