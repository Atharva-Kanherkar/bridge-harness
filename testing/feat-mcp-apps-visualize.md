# Contract: `bridge.visualize` v1 — MCP Apps visuals in chat

Branch `feat/mcp-apps-visualize`. Delivery step 1 of the locked design in the
"Render anything visually in chat with MCP Apps" issue, plus the server and
harness wiring needed for the feature to be reachable from the UI. Locked
before implementation.

## What this changes

A chat agent on Claude, Codex or OpenCode gets one new MCP tool, `visualize`,
served by Bridge itself. The model sends a JSON `VisualSpec` (data, never
code). Bridge validates it, checks that the form fits the data, refuses
repeats, and returns either a short success summary or an `isError` naming
every problem by path so the model can fix it. The transcript draws an
accepted call as a card that holds a sandboxed MCP Apps view, built by Bridge
and rendered from the stored tool input, so reload, fork and replay redraw it
identically.

## Scope of this PR

In:

- `VisualSpec` v1 types, Rust validator (authoritative), TypeScript parser
  (defence in depth), and shared golden fixtures both suites run.
- Form-fit lint and the repeat guard.
- The MCP stdio server as a helper mode of the bundled `bridged` binary
  (`bridged --bridge-mcp-visualize`), with `initialize`, `ping`,
  `tools/list`, `tools/call`, `resources/list` and `resources/read`.
- Attachment to interactive chat sessions on Claude, Codex and OpenCode.
  Workers, read-only workers and briefings never get it.
- The view: a single-file MCP Apps HTML bundle with the `chart` family
  (bar, stacked-bar, grouped-bar, line, area, scatter, heatmap, proportion,
  waterfall, funnel), the `document` family (metric, cards, compare, table,
  callout, steps, checklist, findings, pros-cons, glossary) and the `diagram`
  `grid` form (the existing `DiagramSpec`).
- The host: `McpAppFrame` (sandboxed iframe, CSP, the official
  `@modelcontextprotocol/ext-apps` bridge), the visual card in the
  transcript, follow-up chips that fill the composer, Expand, lazy mounting.
- The when/when-not rubric in the tool description and a pointer in
  `RENDERING_NOTE`.
- The eval dataset, its deterministic subset under `bun run test`, and a
  runner that drives a real harness.

Out, by the locked delivery order (follow-up PRs):

- Value tracing, `visual_verify`, the traced badge (step 2). This PR
  enforces *structural* grounding only: fact-bearing blocks must cite
  declared sources, estimates are drawn as estimates.
- `diagram` forms other than `grid`, and the `time`, `map` and `math`
  families (step 3). The validator answers them with "not available yet"
  and the tool description lists only what this build renders.
- Long-tail chart forms that need the Vega runtime (histogram, box, bubble,
  treemap, sunburst, sankey, slope, dumbbell, candlestick, sparkline-grid).
- Bridge data tools, galleries (step 4); third-party `ui://` apps (step 5);
  Copy as PNG (the frame is cross-origin by design; it needs a view-side
  rasteriser and is deferred).

## Non-negotiable rules

1. The model never supplies HTML, CSS, script, URLs to load, or colours as
   hex. Strings render as text nodes only.
2. The view runs in `<iframe sandbox="allow-scripts">` with no
   `allow-same-origin`, under an injected CSP with `connect-src 'none'`.
3. Every message from the frame is dropped unless its `source` is that
   frame's `contentWindow`.
4. A follow-up chip fills the composer. It never sends.
5. Workers never get the tool.
6. An invalid spec is never drawn as if it were valid: the call fails
   with a path-addressed reason, and the transcript shows a quiet error row.

## Cases

### Spec validation (Rust `mcp_apps::spec`, mirrored by `src/mcp-apps/spec.ts`)

Shared fixtures: `testing/fixtures/visualize/specs.json`, a list of
`{ name, spec, valid, errorPaths? }`. Both suites load it and must agree on
`valid` and on the set of error paths.

- `a_minimal_bar_chart_with_a_source_is_valid`
- `every_available_form_has_a_valid_golden` — one golden per form in scope.
- `version_must_be_1`
- `title_is_required_and_capped_at_80_chars`
- `blocks_must_number_between_1_and_6`
- `an_unknown_family_is_rejected_with_its_path`
- `a_form_outside_its_family_is_rejected` — `family: "chart", form: "steps"`.
- `a_form_not_available_yet_names_the_available_ones` — `family: "map"`.
- `chart_data_must_be_inline_values` — `data.url` is rejected at `blocks[0].vegaLite.data`.
- `vega_lite_config_usermeta_params_transform_and_href_are_rejected`
- `chart_rows_must_be_objects_of_scalars`
- `chart_rows_are_capped_at_5000`
- `an_encoding_field_missing_from_every_row_is_rejected`
- `the_mark_must_agree_with_the_form` — `form: "line"` with `mark: "bar"`.
- `colors_must_come_from_the_enum` — `"#ff0000"` is rejected.
- `fact_blocks_must_cite_declared_sources` — chart, metric, table, compare
  without `sourceIds`; and a `sourceIds` entry with no matching `sources[].id`.
- `findings_items_cite_per_item`
- `web_sources_must_be_http_urls`
- `computed_sources_must_name_existing_inputs`
- `estimate_sources_must_state_a_basis`
- `source_ids_must_be_unique`
- `follow_ups_are_capped_at_4_of_80_chars`
- `grid_diagrams_reuse_diagram_spec` — unknown node in an edge, duplicate ids,
  negative row/col, unknown emphasis/marker.
- `strings_with_control_characters_are_rejected`
- `a_spec_over_256_kib_is_rejected`

### Form-fit lint (Rust `mcp_apps::lint`)

- `fewer_than_three_values_says_use_a_sentence` (proportion needs 2).
- `line_over_unordered_categories_suggests_bar`
- `more_than_six_series_or_parts_is_rejected`
- `scatter_with_fewer_than_8_points_is_rejected`
- `more_than_40_bars_suggests_top_n`
- `a_metric_block_alone_is_rejected`
- `a_small_table_alone_suggests_markdown` — ≤ 4 rows and ≤ 3 columns.
- `a_table_repeating_a_charts_rows_is_rejected`
- `lint_errors_carry_paths_and_a_suggested_fix`

### Repeat guard

- `the_same_form_and_data_twice_is_rejected_the_second_time`
- `a_changed_title_does_not_bypass_the_guard`
- `redraw_true_bypasses_the_guard`
- `the_guard_remembers_only_the_last_10_accepted_calls`
- `a_rejected_call_is_not_remembered`

### MCP server (Rust `mcp_apps::server`)

- `initialize_echoes_a_supported_protocol_version_and_declares_tools_and_resources`
- `tools_list_has_visualize_with_ui_meta_and_read_only_annotations`
- `the_tool_description_lists_exactly_the_available_forms`
- `the_input_schema_uses_no_oneof` — portable across Claude, Codex, OpenCode.
- `tools_call_valid_returns_a_summary` — text only, no `structuredContent`:
  the spec is already the tool input, and some harnesses forward
  structured content to the model as tokens.
- `tools_call_invalid_returns_is_error_listing_every_path`
- `tools_call_unknown_tool_is_a_jsonrpc_error`
- `resources_list_has_ui_bridge_visual`
- `resources_read_returns_the_bundled_view_as_mcp_app_html`
- `notifications_get_no_response_and_unknown_methods_get_32601`
- `malformed_json_lines_get_a_parse_error_and_the_server_keeps_serving`
- `bridged_helper_flag_serves_stdio` (integration: spawn `bridged
  --bridge-mcp-visualize`, initialize, list, call, exit on stdin EOF).

### Harness attachment

- `claude_chat_sessions_get_the_bridge_server` — `sidecar_mcp_servers` for an
  interactive chat contains exactly `bridge` (connectors stay excluded).
- `claude_workers_and_briefings_do_not_get_the_bridge_server`
- `codex_chat_sessions_get_mcp_servers_bridge_overrides`
- `codex_workers_do_not_get_the_overrides`
- `opencode_chat_sessions_get_mcp_bridge_in_config_content`
- `opencode_workers_do_not`
- `no_server_is_attached_when_the_helper_binary_cannot_be_resolved`
- Sidecar `node --test`: chat options carry the stdio server; briefing options
  are unchanged.

### Transcript (`src/transcript`)

- `claude_mcp__bridge__visualize_is_a_visual_call`
- `codex_mcpToolCall_bridge_visualize_is_a_visual_call`
- `opencode_bridge_visualize_is_a_visual_call`
- `another_servers_visualize_is_not` — `mcp__other__visualize`.
- `a_visual_call_stands_alone_and_closes_the_run` — grouping puts it at top
  level between the tool runs before and after it.
- `a_failed_visual_call_reads_as_a_quiet_error_not_a_frame`
- `the_spec_comes_from_input_arguments_or_state_input`

### Host frame and card (jsdom)

- `the_frame_is_sandboxed_allow_scripts_only`
- `the_srcdoc_starts_with_the_csp_meta`
- `messages_from_another_window_are_dropped`
- `tool_input_is_sent_after_the_view_initializes`
- `size_changed_resizes_the_frame`
- `a_theme_flip_resends_host_context`
- `a_follow_up_message_fills_the_composer_and_does_not_send`
- `open_link_goes_through_the_external_open_path`
- `expand_opens_fullscreen_and_escape_closes`
- `an_offscreen_card_unmounts_its_frame_and_keeps_its_height`
- `an_invalid_spec_shows_a_quiet_error_row`

### View (jsdom, per family)

- `each_golden_renders_without_throwing` — every valid golden.
- `strings_render_as_text` — `"<img src=x onerror=alert(1)>"` appears as text,
  and no `img` element is created.
- `estimate_values_are_marked` — a value whose source is `estimate` renders
  dashed or hollow and carries an "est." label.
- `colors_resolve_to_tokens_not_hex`
- `findings_render_numbered_citations_matching_sources`

### Prompt

- `rendering_note_points_to_the_visualize_tool`
- `the_tool_description_carries_the_when_and_when_not_rubric_and_the_form_table`

### Eval

- `testing/evals/visualize/cases.jsonl` holds ≥ 250 labelled cases: ≥ 90
  render, ≥ 90 no-render, ≥ 40 multi-turn, ≥ 30 grounding traps; no
  expected form above 15% of render labels.
- `cases_parse_and_reference_known_forms` (deterministic, in `bun run test`).
- `golden_specs_pass_validation_and_lint` (deterministic).
- Runner `bun run eval:visualize -- --harness claude --limit N` drives the
  real harness with the real tool description and server, and writes a
  results JSON plus a scored table. Run on a sample for this PR and report
  the numbers honestly; full per-harness gates are required from step 2 on.

## Manual / end-to-end

1. `cargo build -p bridged`, then pipe `initialize`, `tools/list` and a
   `tools/call` through `bridged --bridge-mcp-visualize`; the responses
   match the server cases.
2. Live harness check: Claude (`claude -p` with the server in
   `--mcp-config`), Codex (`codex exec -c mcp_servers.bridge…`) and OpenCode
   (`opencode run` with `OPENCODE_CONFIG_CONTENT`) each list and call
   `visualize` without an approval prompt.
3. `bun run dev` mock transcript shows a visual card; screenshots in dark,
   light and Expand go in the PR body.
4. `bun run build` and `bun run test` are green.
