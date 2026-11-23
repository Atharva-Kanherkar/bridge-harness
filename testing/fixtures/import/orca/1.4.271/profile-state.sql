CREATE TABLE profile_state_meta (
    key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL
  );
CREATE TABLE profile_state_documents (
    domain TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL,
    domain_version INTEGER NOT NULL, revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL, content_hash TEXT NOT NULL
  );
CREATE TABLE profile_state_automation_runs_meta (
    domain TEXT PRIMARY KEY NOT NULL,
    presence TEXT NOT NULL,
    domain_version INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL
  );
CREATE TABLE profile_state_automation_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    ordinal INTEGER NOT NULL,
    payload TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
  );
CREATE TABLE profile_state_document_parts_meta (
    domain TEXT PRIMARY KEY NOT NULL,
    domain_version INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    legacy_revision INTEGER
  );
CREATE TABLE profile_state_document_parts (
    domain TEXT NOT NULL,
    part_key TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    payload TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    PRIMARY KEY (domain, part_key)
  );
PRAGMA user_version = 3;
