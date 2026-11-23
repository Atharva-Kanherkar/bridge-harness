CREATE TABLE __drizzle_migrations(id INTEGER PRIMARY KEY, hash text NOT NULL, created_at numeric);
CREATE TABLE `host_agent_configs` (
	`id` text PRIMARY KEY NOT NULL,
	`preset_id` text NOT NULL,
	`label` text NOT NULL,
	`command` text NOT NULL,
	`args_json` text DEFAULT '[]' NOT NULL,
	`prompt_transport` text NOT NULL,
	`prompt_args_json` text DEFAULT '[]' NOT NULL,
	`env_json` text DEFAULT '{}' NOT NULL,
	`display_order` integer NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
, `icon_id` text, `resume_args_json` text DEFAULT '[]' NOT NULL);
CREATE TABLE `host_settings` (
	`id` integer PRIMARY KEY DEFAULT 1 NOT NULL,
	`worktree_base_dir` text,
	`branch_prefix_mode` text,
	`branch_prefix_custom` text
, `default_claude_config_dir` text, `default_codex_home` text);
CREATE TABLE `projects` (
	`id` text PRIMARY KEY NOT NULL,
	`repo_path` text NOT NULL,
	`created_at` integer NOT NULL
, `repo_provider` text, `repo_owner` text, `repo_name` text, `repo_url` text, `remote_name` text, `worktree_base_dir` text, `branch_prefix_mode` text, `branch_prefix_custom` text, `name` text DEFAULT '' NOT NULL, `updated_at` integer DEFAULT 0 NOT NULL, `icon` text, `sparse_checkout_paths` text, `naming_instructions` text, `color` text);
CREATE TABLE `pull_requests` (
	`id` text PRIMARY KEY NOT NULL,
	`project_id` text NOT NULL,
	`repo_provider` text NOT NULL,
	`repo_owner` text NOT NULL,
	`repo_name` text NOT NULL,
	`pr_number` integer NOT NULL,
	`url` text NOT NULL,
	`title` text NOT NULL,
	`state` text NOT NULL,
	`is_draft` integer DEFAULT false NOT NULL,
	`head_branch` text NOT NULL,
	`head_sha` text NOT NULL,
	`review_decision` text,
	`checks_status` text DEFAULT 'none' NOT NULL,
	`checks_json` text DEFAULT '[]' NOT NULL,
	`last_fetched_at` integer,
	`error` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL, `merged_at` integer,
	FOREIGN KEY (`project_id`) REFERENCES `projects`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `terminal_agent_bindings` (
	`terminal_id` text PRIMARY KEY NOT NULL,
	`workspace_id` text NOT NULL,
	`agent_id` text NOT NULL,
	`agent_session_id` text,
	`definition_id` text,
	`started_at` integer NOT NULL,
	`last_event_at` integer NOT NULL,
	`last_event_type` text NOT NULL, `ended_at` integer, `end_reason` text,
	FOREIGN KEY (`terminal_id`) REFERENCES `terminal_sessions`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `terminal_sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`origin_workspace_id` text,
	`status` text DEFAULT 'active' NOT NULL,
	`created_at` integer NOT NULL,
	`last_attached_at` integer,
	`ended_at` integer, `dispose_requested_at` integer,
	FOREIGN KEY (`origin_workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE set null
);
CREATE TABLE `workspace_pull_requests` (
	`workspace_id` text NOT NULL,
	`pull_request_id` text NOT NULL,
	`linked_at` integer NOT NULL,
	PRIMARY KEY(`workspace_id`, `pull_request_id`),
	FOREIGN KEY (`workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`pull_request_id`) REFERENCES `pull_requests`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `workspace_tags` (
	`workspace_id` text NOT NULL,
	`tag` text NOT NULL,
	`created_at` integer NOT NULL,
	PRIMARY KEY(`workspace_id`, `tag`),
	FOREIGN KEY (`workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE "workspaces" (
	`id` text PRIMARY KEY NOT NULL,
	`project_id` text,
	`worktree_path` text NOT NULL,
	`branch` text NOT NULL,
	`head_sha` text,
	`upstream_owner` text,
	`upstream_repo` text,
	`upstream_branch` text,
	`pull_request_id` text,
	`suppressed_pull_request_id` text,
	`name` text DEFAULT '' NOT NULL,
	`type` text DEFAULT 'worktree' NOT NULL,
	`task_id` text,
	`created_by_user_id` text,
	`created_at` integer NOT NULL,
	`updated_at` integer DEFAULT 0 NOT NULL,
	`archived_at` integer, `archive_reason` text,
	FOREIGN KEY (`project_id`) REFERENCES `projects`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`pull_request_id`) REFERENCES `pull_requests`(`id`) ON UPDATE no action ON DELETE set null,
	FOREIGN KEY (`suppressed_pull_request_id`) REFERENCES `pull_requests`(`id`) ON UPDATE no action ON DELETE set null
);
CREATE INDEX `host_agent_configs_display_order_idx` ON `host_agent_configs` (`display_order`);
CREATE INDEX `projects_repo_path_idx` ON `projects` (`repo_path`);
CREATE INDEX `pull_requests_project_id_idx` ON `pull_requests` (`project_id`);
CREATE INDEX `pull_requests_repo_branch_idx` ON `pull_requests` (`repo_provider`,`repo_owner`,`repo_name`,`head_branch`);
CREATE UNIQUE INDEX `pull_requests_repo_pr_unique` ON `pull_requests` (`repo_provider`,`repo_owner`,`repo_name`,`pr_number`);
CREATE INDEX `terminal_agent_bindings_workspace_id_idx` ON `terminal_agent_bindings` (`workspace_id`);
CREATE INDEX `terminal_sessions_origin_workspace_id_idx` ON `terminal_sessions` (`origin_workspace_id`);
CREATE INDEX `terminal_sessions_status_idx` ON `terminal_sessions` (`status`);
CREATE INDEX `workspace_pull_requests_workspace_idx` ON `workspace_pull_requests` (`workspace_id`);
CREATE INDEX `workspace_tags_tag_idx` ON `workspace_tags` (`tag`);
CREATE INDEX `workspaces_archived_at_idx` ON `workspaces` (`archived_at`);
CREATE UNIQUE INDEX `workspaces_one_main_per_project` ON `workspaces` (`project_id`) WHERE type = 'main';
CREATE INDEX `workspaces_project_id_idx` ON `workspaces` (`project_id`);
CREATE INDEX `workspaces_pull_request_id_idx` ON `workspaces` (`pull_request_id`);
CREATE INDEX `workspaces_upstream_ref_idx` ON `workspaces` (`upstream_owner`,`upstream_repo`,`upstream_branch`);
