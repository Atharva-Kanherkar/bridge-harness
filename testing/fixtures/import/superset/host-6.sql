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
);
CREATE TABLE `host_settings` (
	`id` integer PRIMARY KEY DEFAULT 1 NOT NULL,
	`worktree_base_dir` text,
	`branch_prefix_mode` text,
	`branch_prefix_custom` text
);
CREATE TABLE `projects` (
	`id` text PRIMARY KEY NOT NULL,
	`repo_path` text NOT NULL,
	`created_at` integer NOT NULL
, `repo_provider` text, `repo_owner` text, `repo_name` text, `repo_url` text, `remote_name` text, `worktree_base_dir` text, `branch_prefix_mode` text, `branch_prefix_custom` text);
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
	`updated_at` integer NOT NULL,
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
	`last_event_type` text NOT NULL,
	FOREIGN KEY (`terminal_id`) REFERENCES `terminal_sessions`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `terminal_sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`origin_workspace_id` text,
	`status` text DEFAULT 'active' NOT NULL,
	`created_at` integer NOT NULL,
	`last_attached_at` integer,
	`ended_at` integer,
	FOREIGN KEY (`origin_workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE set null
);
CREATE TABLE `workspaces` (
	`id` text PRIMARY KEY NOT NULL,
	`project_id` text NOT NULL,
	`worktree_path` text NOT NULL,
	`branch` text NOT NULL,
	`created_at` integer NOT NULL, `head_sha` text, `pull_request_id` text REFERENCES pull_requests(id), `upstream_owner` text, `upstream_repo` text, `upstream_branch` text,
	FOREIGN KEY (`project_id`) REFERENCES `projects`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE INDEX `host_agent_configs_display_order_idx` ON `host_agent_configs` (`display_order`);
CREATE INDEX `projects_repo_path_idx` ON `projects` (`repo_path`);
CREATE INDEX `pull_requests_project_id_idx` ON `pull_requests` (`project_id`);
CREATE INDEX `pull_requests_repo_branch_idx` ON `pull_requests` (`repo_provider`,`repo_owner`,`repo_name`,`head_branch`);
CREATE UNIQUE INDEX `pull_requests_repo_pr_unique` ON `pull_requests` (`repo_provider`,`repo_owner`,`repo_name`,`pr_number`);
CREATE INDEX `terminal_agent_bindings_workspace_id_idx` ON `terminal_agent_bindings` (`workspace_id`);
CREATE INDEX `terminal_sessions_origin_workspace_id_idx` ON `terminal_sessions` (`origin_workspace_id`);
CREATE INDEX `terminal_sessions_status_idx` ON `terminal_sessions` (`status`);
CREATE INDEX `workspaces_project_id_idx` ON `workspaces` (`project_id`);
CREATE INDEX `workspaces_pull_request_id_idx` ON `workspaces` (`pull_request_id`);
CREATE INDEX `workspaces_upstream_ref_idx` ON `workspaces` (`upstream_owner`,`upstream_repo`,`upstream_branch`);
