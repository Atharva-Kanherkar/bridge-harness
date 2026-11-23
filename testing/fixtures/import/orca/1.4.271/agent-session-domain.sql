CREATE TABLE agent_session_records (
  session_id TEXT PRIMARY KEY,
  record_json TEXT NOT NULL,
  adopted_native_id TEXT GENERATED ALWAYS AS (
    CASE WHEN json_valid(record_json) THEN COALESCE(
      json_extract(record_json,'$.providerHandleChain[#-1].handle.nativeId'),
      json_extract(record_json,'$.providerHandleChain[#-1].handle.sessionId'),
      json_extract(record_json,'$.providerHandleChain[#-1].handle.threadId')) END
  ) VIRTUAL
);
CREATE INDEX agent_session_records_adopted_native ON agent_session_records(adopted_native_id);
CREATE TABLE agent_session_operations (
  operation_key TEXT PRIMARY KEY,
  row_json TEXT NOT NULL,
  operation_id TEXT GENERATED ALWAYS AS (
    CASE WHEN json_valid(row_json) THEN json_extract(row_json, '$.operationId') END
  ) VIRTUAL,
  expires_at INTEGER GENERATED ALWAYS AS (
    CASE WHEN json_valid(row_json) THEN json_extract(row_json, '$.expiresAt') END
  ) VIRTUAL
);
CREATE INDEX agent_session_operations_identity ON agent_session_operations(operation_id);
CREATE INDEX agent_session_operations_expiry ON agent_session_operations(expires_at,operation_key);
CREATE TABLE agent_session_retired_claim_keys (
  key_id TEXT PRIMARY KEY,
  retired_at INTEGER NOT NULL
);
CREATE TABLE agent_session_tabs (
  tab_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  position INTEGER NOT NULL
);
CREATE TABLE agent_session_store_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE agent_session_command_receipts (
  operation_id TEXT NOT NULL,
  session_id TEXT NOT NULL REFERENCES agent_session_records(session_id) ON DELETE CASCADE,
  caller_key TEXT NOT NULL,
  method TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  status TEXT NOT NULL,
  result_json TEXT,
  rejection_json TEXT,
  accepted_at INTEGER NOT NULL,
  PRIMARY KEY(operation_id, caller_key)
);
CREATE TABLE agent_session_attachment_claims (
  upload_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  claimed_at INTEGER NOT NULL,
  PRIMARY KEY(upload_id, session_id)
);
CREATE TABLE agent_session_attachment_sweeps (
  upload_id TEXT NOT NULL PRIMARY KEY,
  started_at INTEGER NOT NULL
);
CREATE TABLE journal_chat_state (
  session_id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  epoch TEXT,
  last_sequence INTEGER NOT NULL DEFAULT 0,
  retained_sequence INTEGER NOT NULL DEFAULT 0,
  highest_fence INTEGER NOT NULL DEFAULT 0,
  context_boundary INTEGER NOT NULL DEFAULT 0,
  last_activity_at INTEGER NOT NULL DEFAULT 0,
  derived_turn_item_id TEXT,
  history_origin_json TEXT,
  accepted_handover_sequence INTEGER NOT NULL DEFAULT 0,
  queue_revision INTEGER NOT NULL DEFAULT 0,
  submissions_revision INTEGER NOT NULL DEFAULT 0,
  queue_latest_stop_json TEXT,
  queue_manual_stop_json TEXT,
  queue_resumed_sequence INTEGER NOT NULL DEFAULT 0,
  queue_reopened_sequence INTEGER NOT NULL DEFAULT 0,
  queue_clear_sequence INTEGER,
  queue_clear_operation_id TEXT,
  queue_clear_lifted INTEGER NOT NULL DEFAULT 0,
  queue_failure_boundary INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE journal_queue_clear_members (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  message_id TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, message_id)
) WITHOUT ROWID;
CREATE TABLE journal_items (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  item_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  sequence INTEGER NOT NULL,
  sequence_index INTEGER NOT NULL,
  owner_fence INTEGER NOT NULL,
  execution_owner INTEGER,
  hidden INTEGER NOT NULL DEFAULT 0,
  root INTEGER NOT NULL DEFAULT 1,
  subagent_row INTEGER NOT NULL DEFAULT 0,
  subagent_group_key TEXT,
  turn_marker INTEGER NOT NULL DEFAULT 0,
  wire_version INTEGER,
  wire_bytes INTEGER,
  turn_item_id TEXT,
  queue_command_turn INTEGER NOT NULL DEFAULT 0,
  queue_failed_outcome INTEGER NOT NULL DEFAULT 0,
  queue_failed_at INTEGER NOT NULL DEFAULT 0,
  queue_failure_witnessed_through INTEGER NOT NULL DEFAULT 0,
  queue_failure_report_turn_item_id TEXT,
  kind TEXT,
  message_role TEXT,
  turn_id TEXT,
  provider_turn_id TEXT,
  codex_thread_id TEXT,
  codex_turn_id TEXT,
  legacy_provider_item_key TEXT,
  opener_item_id TEXT,
  context_clear INTEGER NOT NULL DEFAULT 0,
  goal_transition INTEGER NOT NULL DEFAULT 0,
  producer_agent_id TEXT,
  producer_parent_ref TEXT,
  producer_attempt INTEGER,
  prompt INTEGER NOT NULL DEFAULT 0,
  signed_out INTEGER NOT NULL DEFAULT 0,
  lifecycle_state TEXT,
  live_work INTEGER NOT NULL DEFAULT 0,
  context_window INTEGER NOT NULL DEFAULT 0,
  unverified_end INTEGER NOT NULL DEFAULT 0,
  turn_completed_at REAL,
  command_entry INTEGER NOT NULL DEFAULT 0,
  assistant_prose INTEGER NOT NULL DEFAULT 0,
  turn_scope_kind TEXT,
  payload_json TEXT NOT NULL,
  wire_json TEXT,
  detail_json TEXT,
  outline_source_json TEXT,
  original_stop_note_json TEXT,
  placement_json TEXT, output_blob BLOB,
  PRIMARY KEY(session_id, epoch, item_id)
);
CREATE TABLE journal_settlements (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  settlement_id TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, settlement_id)
) WITHOUT ROWID;
CREATE TABLE journal_input_end_observations (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  source_item_key TEXT NOT NULL,
  observation_json TEXT NOT NULL,
  PRIMARY KEY(session_id,epoch,source_item_key)
) WITHOUT ROWID;
CREATE TABLE journal_item_subagent_names (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  subagent_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  sequence_index INTEGER NOT NULL,
  ordinal INTEGER NOT NULL,
  hidden INTEGER NOT NULL,
  PRIMARY KEY(session_id, epoch, subagent_id, item_id)
) WITHOUT ROWID;
CREATE TABLE journal_item_tombstones (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  item_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  PRIMARY KEY(session_id, epoch, item_id)
) WITHOUT ROWID;
CREATE TABLE journal_removed_turns (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, turn_id)
) WITHOUT ROWID;
CREATE TABLE journal_item_aliases (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  provider_item_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  echo_declared INTEGER NOT NULL DEFAULT 0,
  echo_json TEXT,
  PRIMARY KEY(session_id, epoch, provider_item_id)
) WITHOUT ROWID;
CREATE TABLE journal_submission_attempts (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  message_id TEXT NOT NULL,
  attempt_sequence INTEGER NOT NULL,
  attempt_index INTEGER NOT NULL,
  current INTEGER NOT NULL CHECK(current IN (0,1)),
  item_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  submitted_at INTEGER NOT NULL,
  accepted_sequence INTEGER,
  submitted_sequence INTEGER,
  request_failure INTEGER NOT NULL DEFAULT 0,
  steer_turn_item_id TEXT,
  stopped_history INTEGER NOT NULL DEFAULT 0,
  covers_stopped_history INTEGER NOT NULL DEFAULT 0,
  fingerprint TEXT NOT NULL,
  dispatch_state TEXT NOT NULL,
  provider_item_id TEXT GENERATED ALWAYS AS (
    CASE WHEN json_valid(payload_json) THEN json_extract(payload_json,'$.providerItemId') END
  ) VIRTUAL,
  codex_thread_id TEXT,
  codex_turn_id TEXT,
  queued INTEGER NOT NULL DEFAULT 0,
  dispatch_fence INTEGER NOT NULL DEFAULT 0,
  signed_out INTEGER NOT NULL DEFAULT 0 CHECK(signed_out IN (0,1)),
  recovered INTEGER NOT NULL DEFAULT 0 CHECK(recovered IN (0,1)),
  candidate INTEGER NOT NULL DEFAULT 0,
  admitted_personal INTEGER NOT NULL DEFAULT 0 CHECK(admitted_personal IN (0,1)),
  echo_observed INTEGER NOT NULL DEFAULT 0 CHECK(echo_observed IN (0,1)),
  admission_body_json TEXT,
  admission_family_id TEXT,
  handover_fence INTEGER,
  facts_json TEXT NOT NULL DEFAULT '{}',
  outline_json TEXT,
  payload_json TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, message_id, attempt_sequence, attempt_index)
);
CREATE UNIQUE INDEX journal_submission_head ON journal_submission_attempts(session_id, epoch, message_id) WHERE current=1;
CREATE UNIQUE INDEX journal_submission_item ON journal_submission_attempts(session_id, epoch, item_id) WHERE current=1;
CREATE TABLE journal_send_receipts (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  message_id TEXT NOT NULL,
  cursor_sequence INTEGER NOT NULL DEFAULT 0,
  payload_json TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, message_id)
) WITHOUT ROWID;
CREATE TABLE journal_provider_activity (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  fence INTEGER NOT NULL,
  last_activity_at INTEGER NOT NULL,
  PRIMARY KEY(session_id, epoch, fence)
) WITHOUT ROWID;
CREATE TABLE journal_events (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  saved_at INTEGER NOT NULL,
  changes_json TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, sequence)
);
CREATE TABLE journal_queue_cards (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  message_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  state TEXT NOT NULL,
  position REAL NOT NULL,
  command_turn_item_id TEXT,
  payload_json TEXT NOT NULL,
  PRIMARY KEY(session_id, epoch, message_id)
);
CREATE TABLE journal_edit_leases (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  caller_key TEXT NOT NULL,
  edit_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  card_revision INTEGER NOT NULL,
  expires_at REAL NOT NULL,
  PRIMARY KEY(session_id, epoch, owner_id, caller_key, edit_id)
) WITHOUT ROWID;
CREATE TABLE journal_maintenance (
  id INTEGER PRIMARY KEY CHECK(id=1),
  compacted_at INTEGER NOT NULL
);
CREATE TABLE journal_outline_cache (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  source_sequence INTEGER NOT NULL,
  source_revision INTEGER NOT NULL,
  budget_bytes INTEGER NOT NULL,
  outline_json TEXT NOT NULL,
  PRIMARY KEY(session_id,epoch)
) WITHOUT ROWID;
CREATE INDEX journal_submission_sending ON journal_submission_attempts(session_id,epoch,ordinal,message_id)
  WHERE current=1 AND dispatch_state='pending';
CREATE INDEX idx_agent_session_clear_successor ON agent_session_records(
  CASE WHEN json_valid(record_json) THEN json_extract(record_json, '$.conversationCommand.replacementSessionId') END);
CREATE INDEX agent_session_command_receipts_session ON agent_session_command_receipts(session_id);
CREATE INDEX journal_chat_ready ON journal_chat_state(session_id) WHERE epoch IS NOT NULL;
CREATE INDEX journal_items_ordinal ON journal_items(session_id,epoch,ordinal);
CREATE INDEX journal_items_unsettled ON journal_items(session_id,epoch,ordinal,item_id)
  WHERE hidden=0 AND (lifecycle_state IN ('running','pending') OR live_work=1);
CREATE INDEX journal_items_context_window ON journal_items(session_id,epoch) WHERE hidden=0 AND context_window=1;
CREATE INDEX journal_items_unverified_end ON journal_items(session_id,epoch,ordinal,item_id) WHERE hidden=0 AND unverified_end=1;
CREATE INDEX journal_items_unverifiable_turn ON journal_items(session_id,epoch,ordinal,item_id) WHERE hidden=0 AND kind IN ('turn','status') AND lifecycle_state='unverifiable';
CREATE INDEX journal_items_interrupted_root ON journal_items(session_id,epoch,turn_completed_at,sequence,sequence_index,ordinal,item_id)
  WHERE hidden=0 AND root=1 AND kind IN ('turn','status') AND lifecycle_state='interrupted' AND turn_completed_at IS NOT NULL;
CREATE INDEX journal_item_subagent_names_item ON journal_item_subagent_names(session_id, epoch, item_id);
CREATE INDEX journal_item_subagent_names_visible ON journal_item_subagent_names(
  session_id, epoch, subagent_id, sequence, sequence_index, ordinal) WHERE hidden=0;
CREATE INDEX journal_item_aliases_target ON journal_item_aliases(session_id, epoch, item_id);
CREATE INDEX journal_submission_echo ON journal_submission_attempts(session_id, epoch, fingerprint, submitted_at, ordinal) WHERE current=1;
CREATE INDEX journal_submission_person_acceptance ON journal_submission_attempts(session_id, epoch, accepted_sequence DESC) WHERE dispatch_state='accepted' AND admitted_personal=1;
CREATE INDEX journal_submission_family ON journal_submission_attempts(session_id,epoch,admission_family_id,attempt_sequence,attempt_index,ordinal) WHERE current=1;
CREATE INDEX journal_submission_ordinal ON journal_submission_attempts(session_id,epoch,ordinal);
CREATE INDEX journal_submission_recovery ON journal_submission_attempts(session_id,epoch,ordinal,message_id)
  WHERE current=1 AND (dispatch_state='pending' OR (dispatch_state='unknown' AND recovered=0));
CREATE INDEX journal_submission_acceptance ON journal_submission_attempts(session_id, epoch, accepted_sequence DESC) WHERE dispatch_state='accepted';
CREATE INDEX journal_submission_candidates ON journal_submission_attempts(session_id,epoch,ordinal,message_id) WHERE current=1 AND candidate=1;
CREATE INDEX journal_submission_unsettled ON journal_submission_attempts(session_id,epoch,ordinal,message_id) WHERE current=1 AND dispatch_state IN ('pending','unknown');
CREATE INDEX journal_submission_status_accepted ON journal_submission_attempts(session_id,epoch,submitted_at,ordinal,message_id) WHERE current=1 AND dispatch_state='accepted';
CREATE INDEX journal_submission_status_failed ON journal_submission_attempts(session_id,epoch,accepted_sequence,ordinal,message_id) WHERE current=1 AND request_failure=1;
CREATE INDEX journal_submission_non_refused ON journal_submission_attempts(session_id,epoch,accepted_sequence,ordinal,message_id) WHERE current=1 AND dispatch_state<>'rejected' AND accepted_sequence IS NOT NULL;
CREATE INDEX journal_submission_steer ON journal_submission_attempts(session_id,epoch,steer_turn_item_id,accepted_sequence,ordinal) WHERE current=1 AND steer_turn_item_id IS NOT NULL;
CREATE INDEX journal_submission_submitted ON journal_submission_attempts(session_id,epoch,submitted_sequence) WHERE current=1 AND submitted_sequence IS NOT NULL;
CREATE INDEX journal_submission_stopped ON journal_submission_attempts(session_id,epoch,ordinal,message_id) WHERE current=1 AND stopped_history=1;
CREATE INDEX journal_submission_carriers ON journal_submission_attempts(session_id,epoch,ordinal,message_id) WHERE current=1 AND covers_stopped_history=1;
CREATE INDEX journal_send_receipt_sequence ON journal_send_receipts(session_id,epoch,cursor_sequence,message_id);
CREATE INDEX journal_queue_order ON journal_queue_cards(session_id, epoch, position, message_id)
  WHERE state IN ('waiting','returned');
CREATE INDEX journal_queue_last_position ON journal_queue_cards(session_id, epoch, position);
CREATE INDEX journal_queue_source_metadata ON journal_queue_cards(session_id,epoch,message_id,
  CASE WHEN json_valid(payload_json) THEN octet_length(json_extract(payload_json,'$.body')) END,
  CASE WHEN json_valid(payload_json) THEN json_extract(payload_json,'$.queuedAt.sequence') END);
CREATE INDEX journal_queue_returned_commands ON journal_queue_cards(session_id, epoch, command_turn_item_id)
  WHERE state='returned';
CREATE INDEX journal_edit_leases_message ON journal_edit_leases(session_id, epoch, message_id);
CREATE INDEX journal_edit_leases_deadline ON journal_edit_leases(session_id, epoch, expires_at);
CREATE INDEX journal_items_root_position ON journal_items(session_id, epoch, sequence, sequence_index, ordinal)
  WHERE hidden=0 AND root=1;
CREATE INDEX journal_items_queue_eligible_turn ON journal_items(session_id, epoch, sequence, sequence_index, ordinal)
  WHERE hidden=0 AND turn_marker=1 AND (queue_command_turn=0 OR queue_failed_outcome=0 OR queue_failed_at>0);
CREATE INDEX journal_items_turn_marker ON journal_items(session_id, epoch, sequence, sequence_index, ordinal)
  WHERE hidden=0 AND turn_marker=1;
CREATE INDEX journal_items_raw_turn_marker ON journal_items(session_id,epoch,sequence,ordinal)
  WHERE turn_marker=1;
CREATE INDEX journal_items_visible_position ON journal_items(session_id, epoch, sequence, sequence_index, ordinal)
  WHERE hidden=0;
CREATE INDEX journal_items_queue_failure_report ON journal_items(session_id, epoch, queue_failure_report_turn_item_id)
  WHERE hidden=0 AND queue_failure_report_turn_item_id IS NOT NULL;
CREATE INDEX journal_items_stop_turn ON journal_items(session_id, epoch, turn_item_id)
  WHERE hidden=0 AND turn_item_id IS NOT NULL;
CREATE INDEX journal_items_original_stop_note ON journal_items(session_id,epoch,ordinal)
  WHERE hidden=0 AND original_stop_note_json IS NOT NULL;
CREATE INDEX journal_items_teardown_turn ON journal_items(session_id,epoch,sequence,sequence_index,ordinal)
  WHERE hidden=0 AND root=1 AND turn_id IS NOT NULL AND queue_command_turn=0;
CREATE INDEX journal_events_saved_at ON journal_events(saved_at,session_id,epoch,sequence);
CREATE INDEX journal_items_kind ON journal_items(session_id, epoch, kind, ordinal) WHERE hidden=0;
CREATE INDEX journal_items_user_message ON journal_items(session_id, epoch, ordinal)
  WHERE hidden=0 AND kind='message' AND message_role='user';
CREATE INDEX journal_items_named_turn ON journal_items(session_id, epoch, turn_id, sequence, sequence_index, ordinal)
  WHERE hidden=0 AND turn_id IS NOT NULL;
CREATE INDEX journal_items_provider_turn ON journal_items(session_id,epoch,provider_turn_id,sequence,sequence_index,ordinal)
  WHERE provider_turn_id IS NOT NULL;
CREATE INDEX journal_items_codex_turn ON journal_items(session_id,epoch,codex_thread_id,codex_turn_id,sequence,sequence_index,ordinal)
  WHERE hidden=0 AND codex_turn_id IS NOT NULL;
CREATE INDEX journal_items_visible_provider_key ON journal_items(session_id,epoch,legacy_provider_item_key)
  WHERE hidden=0 AND legacy_provider_item_key IS NOT NULL;
CREATE INDEX journal_submission_codex_turn ON journal_submission_attempts(session_id,epoch,codex_thread_id,codex_turn_id,item_id)
  WHERE current=1 AND dispatch_state='accepted';
CREATE INDEX journal_items_opener ON journal_items(session_id, epoch, opener_item_id, ordinal)
  WHERE hidden=0 AND opener_item_id IS NOT NULL;
CREATE INDEX journal_items_context_clear ON journal_items(session_id, epoch, sequence)
  WHERE hidden=0 AND context_clear=1;
CREATE INDEX journal_items_raw_context_clear ON journal_items(session_id,epoch,sequence)
  WHERE context_clear=1;
CREATE INDEX journal_items_goal_transition ON journal_items(session_id, epoch, sequence, ordinal)
  WHERE hidden=0 AND goal_transition=1;
CREATE INDEX journal_items_producer_parent ON journal_items(session_id, epoch, producer_parent_ref, ordinal, producer_agent_id)
  WHERE hidden=0 AND producer_parent_ref IS NOT NULL;
CREATE INDEX journal_items_producer_attempt ON journal_items(session_id, epoch, producer_agent_id, producer_attempt)
  WHERE hidden=0 AND producer_agent_id IS NOT NULL;
CREATE INDEX journal_items_status_turn ON journal_items(session_id,epoch,sequence,sequence_index,ordinal) WHERE hidden=0 AND turn_id IS NOT NULL;
CREATE INDEX journal_items_status_prompt ON journal_items(session_id,epoch,sequence,sequence_index,ordinal) WHERE hidden=0 AND prompt=1;
CREATE INDEX journal_items_status_prose ON journal_items(session_id,epoch,sequence,sequence_index,ordinal) WHERE hidden=0 AND root=1 AND message_role='assistant' AND command_entry=0 AND assistant_prose=1;
CREATE INDEX journal_items_status_pending ON journal_items(session_id,epoch,sequence,sequence_index,ordinal) WHERE hidden=0 AND kind IN ('approval','question') AND lifecycle_state='pending';
CREATE INDEX journal_items_status_tool ON journal_items(session_id,epoch,sequence,sequence_index,ordinal) WHERE hidden=0 AND root=1 AND kind IN ('tool-call','diff');
CREATE INDEX journal_items_status_assistant ON journal_items(session_id,epoch) WHERE hidden=0 AND message_role='assistant';
CREATE INDEX journal_items_signed_out ON journal_items(session_id, epoch, COALESCE(execution_owner,owner_fence))
  WHERE hidden=0 AND root=1 AND signed_out=1;
CREATE INDEX journal_submission_accepted_position ON journal_submission_attempts(session_id, epoch, accepted_sequence, ordinal)
  WHERE current=1 AND accepted_sequence IS NOT NULL;
CREATE INDEX journal_submission_signed_out ON journal_submission_attempts(session_id,epoch,dispatch_fence)
  WHERE current=1 AND signed_out=1;
CREATE INDEX journal_submission_latest_fence ON journal_submission_attempts(session_id,epoch,dispatch_fence,submitted_at,ordinal) WHERE current=1;
CREATE INDEX journal_submission_order ON journal_submission_attempts(session_id, epoch, submitted_at, ordinal, message_id)
  WHERE current=1;
CREATE TABLE journal_text_tail (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  item_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  text TEXT NOT NULL,
  -- What is saved through this row, so the next save reads this row alone.
  saved_chars INTEGER NOT NULL,
  folded_chars INTEGER NOT NULL,
  saved_hash TEXT NOT NULL,
  skeleton TEXT NOT NULL,
  -- The item's turn, so a finished turn finds its own tails without visiting other turns'.
  turn_item_id TEXT,
  PRIMARY KEY(session_id, epoch, item_id, seq)
) WITHOUT ROWID;
CREATE TRIGGER journal_text_tail_rewritten AFTER UPDATE OF payload_json ON journal_items
BEGIN
  DELETE FROM journal_text_tail
  WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND item_id=OLD.item_id;
END;
CREATE TRIGGER journal_text_tail_removed AFTER DELETE ON journal_items
BEGIN
  DELETE FROM journal_text_tail
  WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND item_id=OLD.item_id;
END;
CREATE INDEX journal_text_tail_turn
    ON journal_text_tail(session_id, epoch, turn_item_id, item_id) WHERE turn_item_id IS NOT NULL;
CREATE TABLE journal_connector_replies (
  account TEXT NOT NULL,
  delivery_id TEXT NOT NULL,
  source TEXT NOT NULL,
  external_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  message_id TEXT NOT NULL,
  queued INTEGER NOT NULL,
  outcome TEXT,
  answer_delivery_id TEXT,
  turn_item_id TEXT,
  prose TEXT,
  failure_json TEXT,
  links_json TEXT,
  filled_at INTEGER,
  sent_at INTEGER,
  took_ms INTEGER,
  cut_turn_id TEXT,
  cut_kind TEXT,
  claim_message_id TEXT,
  PRIMARY KEY(account, delivery_id)
) WITHOUT ROWID;
CREATE INDEX journal_connector_replies_open
  ON journal_connector_replies(session_id, outcome, sent_at);
CREATE TABLE journal_retire_pending (
        session_id TEXT PRIMARY KEY,
        generation INTEGER NOT NULL
      ) WITHOUT ROWID;
CREATE TRIGGER journal_retire_pending_head_insert AFTER INSERT ON journal_chat_state
      WHEN NEW.epoch IS NOT NULL
      BEGIN
        INSERT INTO journal_retire_pending(session_id,generation) VALUES(NEW.session_id,1)
        ON CONFLICT(session_id) DO UPDATE SET generation=generation+1;
      END;
CREATE TRIGGER journal_retire_pending_head_epoch
      AFTER UPDATE OF epoch ON journal_chat_state
      WHEN NEW.epoch IS NOT NULL AND NEW.epoch IS NOT OLD.epoch
      BEGIN
        INSERT INTO journal_retire_pending(session_id,generation) VALUES(NEW.session_id,1)
        ON CONFLICT(session_id) DO UPDATE SET generation=generation+1;
      END;
CREATE TRIGGER journal_retire_pending_item_epoch AFTER INSERT ON journal_items
      WHEN EXISTS (SELECT 1 FROM journal_chat_state
        WHERE session_id=NEW.session_id AND epoch IS NOT NULL AND epoch IS NOT NEW.epoch)
      BEGIN
        INSERT INTO journal_retire_pending(session_id,generation) VALUES(NEW.session_id,1)
        ON CONFLICT(session_id) DO UPDATE SET generation=generation+1;
      END;
CREATE TABLE pr_chat_actions (
  chat_id         TEXT    NOT NULL,
  anchor_item_id  TEXT    NOT NULL,
  anchor_revision INTEGER NOT NULL,
  forge_host      TEXT    NOT NULL,
  owner           TEXT    NOT NULL,
  repo            TEXT    NOT NULL,
  number          INTEGER NOT NULL,
  kind            TEXT    NOT NULL,
  head_sha        TEXT,
  observed_at     INTEGER NOT NULL,
  PRIMARY KEY (chat_id, anchor_item_id, forge_host, owner, repo, number, kind)
);
CREATE INDEX pr_chat_actions_by_pr
  ON pr_chat_actions (forge_host, owner, repo, number);
CREATE TABLE pr_check_events (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  pr_key          TEXT    NOT NULL,
  head_sha        TEXT    NOT NULL,
  occurrence      INTEGER NOT NULL,
  verdict         TEXT    NOT NULL,
  completed_at    INTEGER NOT NULL,
  observed_at     INTEGER NOT NULL,
  is_current_head INTEGER NOT NULL,
  UNIQUE (pr_key, head_sha, occurrence)
);
CREATE TABLE pr_event_deliveries (
  chat_id  TEXT    NOT NULL,
  event_id INTEGER NOT NULL,
  state    TEXT    NOT NULL,
  item_id  TEXT,
  PRIMARY KEY (chat_id, event_id)
);
CREATE TABLE pr_pending_heads (
  pr_key        TEXT    NOT NULL,
  head_sha      TEXT    NOT NULL,
  first_seen_at INTEGER NOT NULL,
  moved_at      INTEGER,
  expires_at    INTEGER,
  PRIMARY KEY (pr_key, head_sha)
);
CREATE TABLE pr_pending_actions (
  chat_id             TEXT    NOT NULL,
  anchor_item_id      TEXT    NOT NULL,
  anchor_revision     INTEGER NOT NULL,
  evidence_json       TEXT    NOT NULL,
  finished_at         INTEGER NOT NULL,
  resolve_deadline_at INTEGER NOT NULL,
  verify_deadline_at  INTEGER NOT NULL,
  next_attempt_at     INTEGER NOT NULL,
  attempts            INTEGER NOT NULL,
  PRIMARY KEY (chat_id, anchor_item_id)
);
CREATE INDEX pr_pending_actions_by_due ON pr_pending_actions (next_attempt_at);
CREATE TABLE pr_check_contexts (
  pr_key       TEXT    NOT NULL,
  head_sha     TEXT    NOT NULL,
  context      TEXT    NOT NULL,
  last_state   TEXT,
  last_run_id  TEXT,
  pending_seen INTEGER NOT NULL,
  PRIMARY KEY (pr_key, head_sha, context)
);
CREATE INDEX journal_items_async_question
  ON journal_items(session_id,epoch,sequence,sequence_index,ordinal)
  WHERE hidden=0 AND root=1 AND message_role='assistant' AND command_entry=0 AND instr(payload_json,'request_user_input_async')>0;
CREATE INDEX journal_items_person_prompt
  ON journal_items(session_id,epoch,sequence,sequence_index,ordinal)
  WHERE hidden=0 AND prompt=1 AND (CASE WHEN json_valid(payload_json) THEN json_extract(payload_json,'$.body.from') END) IS NULL;
CREATE TABLE journal_ended_turn_rows (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  rows INTEGER NOT NULL,
  PRIMARY KEY(session_id, epoch, turn_id)
) WITHOUT ROWID;
CREATE TABLE journal_ended_turn_counts (
  session_id TEXT NOT NULL,
  epoch TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY(session_id, epoch)
) WITHOUT ROWID;
CREATE TRIGGER journal_ended_turns_insert AFTER INSERT ON journal_items
  WHEN (NEW.hidden=0 AND NEW.turn_marker=1 AND NEW.context_clear=0
    AND NEW.lifecycle_state IS NOT 'running' AND NEW.turn_id IS NOT NULL) BEGIN 
  INSERT INTO journal_ended_turn_counts(session_id,epoch,count)
    SELECT NEW.session_id,NEW.epoch,1 WHERE NOT EXISTS(SELECT 1 FROM journal_ended_turn_rows
      WHERE session_id=NEW.session_id AND epoch=NEW.epoch AND turn_id=NEW.turn_id)
    ON CONFLICT(session_id,epoch) DO UPDATE SET count=count+1;
  INSERT INTO journal_ended_turn_rows(session_id,epoch,turn_id,rows)
    VALUES(NEW.session_id,NEW.epoch,NEW.turn_id,1)
    ON CONFLICT(session_id,epoch,turn_id) DO UPDATE SET rows=rows+1; END;
CREATE TRIGGER journal_ended_turns_delete AFTER DELETE ON journal_items
  WHEN (OLD.hidden=0 AND OLD.turn_marker=1 AND OLD.context_clear=0
    AND OLD.lifecycle_state IS NOT 'running' AND OLD.turn_id IS NOT NULL) BEGIN 
  UPDATE journal_ended_turn_rows SET rows=rows-1
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id;
  UPDATE journal_ended_turn_counts SET count=count-1
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND EXISTS(SELECT 1 FROM journal_ended_turn_rows
      WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id AND rows<=0);
  DELETE FROM journal_ended_turn_rows
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id AND rows<=0;
  DELETE FROM journal_ended_turn_counts
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND count<=0; END;
CREATE TRIGGER journal_ended_turns_update_old AFTER UPDATE OF session_id,epoch,hidden,turn_marker,context_clear,lifecycle_state,turn_id ON journal_items
  WHEN (OLD.hidden=0 AND OLD.turn_marker=1 AND OLD.context_clear=0
    AND OLD.lifecycle_state IS NOT 'running' AND OLD.turn_id IS NOT NULL) AND NOT ((NEW.hidden=0 AND NEW.turn_marker=1 AND NEW.context_clear=0
    AND NEW.lifecycle_state IS NOT 'running' AND NEW.turn_id IS NOT NULL) AND (NEW.session_id=OLD.session_id AND NEW.epoch=OLD.epoch AND NEW.turn_id=OLD.turn_id)) BEGIN 
  UPDATE journal_ended_turn_rows SET rows=rows-1
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id;
  UPDATE journal_ended_turn_counts SET count=count-1
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND EXISTS(SELECT 1 FROM journal_ended_turn_rows
      WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id AND rows<=0);
  DELETE FROM journal_ended_turn_rows
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND turn_id=OLD.turn_id AND rows<=0;
  DELETE FROM journal_ended_turn_counts
    WHERE session_id=OLD.session_id AND epoch=OLD.epoch AND count<=0; END;
CREATE TRIGGER journal_ended_turns_update_new AFTER UPDATE OF session_id,epoch,hidden,turn_marker,context_clear,lifecycle_state,turn_id ON journal_items
  WHEN (NEW.hidden=0 AND NEW.turn_marker=1 AND NEW.context_clear=0
    AND NEW.lifecycle_state IS NOT 'running' AND NEW.turn_id IS NOT NULL) AND NOT ((OLD.hidden=0 AND OLD.turn_marker=1 AND OLD.context_clear=0
    AND OLD.lifecycle_state IS NOT 'running' AND OLD.turn_id IS NOT NULL) AND (NEW.session_id=OLD.session_id AND NEW.epoch=OLD.epoch AND NEW.turn_id=OLD.turn_id)) BEGIN 
  INSERT INTO journal_ended_turn_counts(session_id,epoch,count)
    SELECT NEW.session_id,NEW.epoch,1 WHERE NOT EXISTS(SELECT 1 FROM journal_ended_turn_rows
      WHERE session_id=NEW.session_id AND epoch=NEW.epoch AND turn_id=NEW.turn_id)
    ON CONFLICT(session_id,epoch) DO UPDATE SET count=count+1;
  INSERT INTO journal_ended_turn_rows(session_id,epoch,turn_id,rows)
    VALUES(NEW.session_id,NEW.epoch,NEW.turn_id,1)
    ON CONFLICT(session_id,epoch,turn_id) DO UPDATE SET rows=rows+1; END;
PRAGMA user_version = 2;
