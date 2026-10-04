// Presets: the agent definitions, as a list and one detail page each.
//
// The old page drew a third sidebar of its own, so content started 440px into
// the window and the column jumped every time you left the page. It is now the
// same list-and-detail shape as Harnesses and Prompts, in the same column.
//
// Saving follows the screen's one rule, with one carve-out that the rule
// implies rather than contradicts: a preset that has never been saved has no
// stored record to write one field into, so on a new preset every control
// dirties the draft and the bar reads "Create agent". On an existing preset,
// switches and selects write the stored record with exactly one field replaced.

import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
import type { AdapterDescriptor, AgentDefinition, AgentRole, ReasoningEffort } from "../../types";
import { HarnessMark } from "../harnessMarks";
import {
  Field, GhostButton, SaveBar, Select, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow,
  StatusPill, Switch, TextArea, TextButton, useSavedFlash,
} from "./kit";
import type { ModelOption } from "./HarnessesPage";

const ROLES: { id: AgentRole; label: string }[] = [
  { id: "orchestrator", label: "Main chat" }, { id: "research", label: "Research" },
  { id: "implementation", label: "Write code" }, { id: "verification", label: "Check results" },
  { id: "planning", label: "Planning" }, { id: "documentation", label: "Write documentation" },
];
const EFFORTS: ReasoningEffort[] = ["low", "medium", "high", "xhigh"];

export function newAgent(): AgentDefinition {
  return {
    id: "", name: "New agent", description: "", role: "orchestrator", harness: "bridge",
    model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: false,
    isBuiltIn: false, createdAt: "", updatedAt: "",
  };
}

export function PresetsPage({
  agents, adapters, modelOptions, busy, draft, supportsRole,
  onOpen, onClose, onNew, onDraft, onSave, onRemove, onMakeDefault, onError,
}: {
  agents: AgentDefinition[];
  adapters: AdapterDescriptor[];
  modelOptions: ModelOption[];
  busy: boolean;
  /** The preset being edited, or undefined on the list page. */
  draft?: AgentDefinition;
  supportsRole: (adapter: AdapterDescriptor, role: string) => boolean;
  onOpen: (agent: AgentDefinition) => void;
  onClose: () => void;
  onNew: () => void;
  onDraft: (next: AgentDefinition) => void;
  onSave: (next: AgentDefinition) => Promise<void>;
  onRemove: (agent: AgentDefinition) => void;
  onMakeDefault: (agent: AgentDefinition) => void;
  onError: (message: string) => void;
}) {
  const mine = agents.filter(agent => !agent.isBuiltIn);
  const builtIn = agents.filter(agent => agent.isBuiltIn);
  const row = (agent: AgentDefinition) => <SettingsRow
    key={agent.id}
    lead={<HarnessMark harness={agent.harness} size={14} />}
    label={agent.name}
    openLabel={`Edit ${agent.name}`}
    description={`${agent.role} · ${agent.harness}`}
    onOpen={() => onOpen(agent)}
    control={<>
      {agent.isDefault && <StatusPill tone="info">Default</StatusPill>}
      {!agent.enabled && <StatusPill>Off</StatusPill>}
    </>}
  />;
  if (draft) {
    return <PresetDetail
      draft={draft}
      stored={agents.find(item => item.id === draft.id)}
      adapters={adapters}
      modelOptions={modelOptions}
      busy={busy}
      supportsRole={supportsRole}
      onBack={onClose}
      onDraft={onDraft}
      onSave={onSave}
      onRemove={onRemove}
      onMakeDefault={onMakeDefault}
      onError={onError}
    />;
  }

  return <SettingsPage
    title="Agents"
    description="Save a model and instructions you want to reuse. Start a new chat with one from the agent menu, or type # in a chat."
    action={<GhostButton onClick={onNew}><Plus size={12} strokeWidth={1.7} aria-hidden="true" />New agent</GhostButton>}
  >
    <SettingsGroup label="Your agents" note={`${mine.length} total`}>
      {mine.length === 0 && <SettingsRow label="No agents yet" description="Create one when you want your own reusable instructions. Bridge's built-in agents are below." />}
      {mine.map(row)}
    </SettingsGroup>
    <SettingsGroup label="Bridge defaults" note={`${builtIn.length} built in`}>
      {builtIn.map(row)}
    </SettingsGroup>
  </SettingsPage>;
}

function PresetDetail({
  draft, stored, adapters, modelOptions, busy, supportsRole,
  onBack, onDraft, onSave, onRemove, onMakeDefault, onError,
}: {
  draft: AgentDefinition;
  stored?: AgentDefinition;
  adapters: AdapterDescriptor[];
  modelOptions: ModelOption[];
  busy: boolean;
  supportsRole: (adapter: AdapterDescriptor, role: string) => boolean;
  onBack: () => void;
  onDraft: (next: AgentDefinition) => void;
  onSave: (next: AgentDefinition) => Promise<void>;
  onRemove: (agent: AgentDefinition) => void;
  onMakeDefault: (agent: AgentDefinition) => void;
  onError: (message: string) => void;
}) {
  const [isFlashed, flash] = useSavedFlash();
  const isNew = !draft.id;
  const dirty = isNew || !stored
    || draft.name !== stored.name
    || draft.description !== stored.description
    || draft.systemPrompt !== stored.systemPrompt;

  const harnessOptions = useMemo(() => [
    { value: "bridge", label: "Bridge chooses", lead: <HarnessMark harness="bridge" size={12} /> },
    ...adapters.filter(adapter => supportsRole(adapter, draft.role)).map(adapter => ({
      value: adapter.id,
      label: adapter.label,
      lead: <HarnessMark harness={adapter.id} size={12} />,
    })),
  ], [adapters, supportsRole, draft.role]);

  const models = useMemo(
    () => modelOptions.filter(option => option.adapter === draft.harness).map(option => ({ value: option.id, label: option.label })),
    [modelOptions, draft.harness],
  );

  /**
   * A switch or select on an existing preset persists at once, writing the
   * stored record with one field replaced so an unsaved name never rides along.
   * On a preset with no id there is nothing to write into, so it only drafts.
   */
  const set = (patch: Partial<AgentDefinition>, key: string) => {
    onDraft({ ...draft, ...patch });
    if (isNew || !stored) return;
    void onSave({ ...stored, ...patch }).then(() => flash(key)).catch(error =>
      onError(error instanceof Error ? error.message : String(error)));
  };

  return <SettingsPage
    title={isNew ? "New agent" : draft.name}
    breadcrumb={[{ label: "Agents", onClick: onBack }, { label: isNew ? "New agent" : draft.name }]}
    description={isNew
      ? "Your agent. You can delete it at any time."
      : draft.isBuiltIn ? "Built-in Bridge role. Reset restores its original instructions and choices." : "Your agent. You can delete it at any time."}
    action={!isNew && <>
      {draft.role === "orchestrator" && !draft.isDefault && <TextButton
        disabled={busy || !draft.enabled}
        onClick={() => onMakeDefault(draft)}
      >Make default</TextButton>}
      <TextButton tone="destructive" disabled={busy} onClick={() => onRemove(draft)}>
        {draft.isBuiltIn ? "Reset" : "Delete"}
      </TextButton>
    </>}
  >
    <SettingsGroup label="Setup">
      <SettingsRow
        label="Enabled"
        saved={isFlashed("enabled")}
        control={<Switch
          label="Setup enabled"
          checked={draft.enabled}
          disabled={busy}
          onChange={next => set({ enabled: next }, "enabled")}
        />}
      />
      <SettingsRow
        label="Name"
        control={<Field label="Setup name" value={draft.name} disabled={busy} onChange={value => onDraft({ ...draft, name: value })} />}
      />
      <SettingsRow
        label="Description"
        control={<Field label="Setup description" value={draft.description ?? ""} disabled={busy} onChange={value => onDraft({ ...draft, description: value })} />}
      />
      <SettingsRow
        label="What it does"
        saved={isFlashed("role")}
        control={<Select
          label="What this agent does"
          value={draft.role}
          disabled={busy}
          width="w-44"
          options={ROLES.map(role => ({ value: role.id, label: role.label }))}
          onChange={value => {
            const role = value as AgentRole;
            // A runtime that cannot take the new role falls back to Bridge,
            // rather than being left naming a role it will refuse.
            const current = adapters.find(adapter => adapter.id === draft.harness);
            const keeps = draft.harness === "bridge" || (current && supportsRole(current, role));
            set({ role, harness: keeps ? draft.harness : "bridge", model: keeps ? draft.model : null }, "role");
          }}
        />}
      />
    </SettingsGroup>

    <SettingsGroup label="Coding agent & model">
      <SettingsRow
        label="Coding agent"
        saved={isFlashed("harness")}
        control={<Select
          label="Coding agent for this agent"
          value={draft.harness}
          disabled={busy}
          options={harnessOptions}
          onChange={value => set({ harness: value as AgentDefinition["harness"], model: null }, "harness")}
        />}
      />
      <SettingsRow
        label="Model"
        saved={isFlashed("model")}
        control={<Select
          label="Model for this agent"
          value={draft.model ?? ""}
          disabled={busy || draft.harness === "bridge"}
          options={[{ value: "", label: "Provider default" }, ...models]}
          onChange={value => set({ model: value || null }, "model")}
        />}
      />
      <SettingsRow
        label="Thinking level"
        saved={isFlashed("effort")}
        control={<Select
          label="Thinking level for this agent"
          value={draft.effort}
          disabled={busy}
          width="w-40"
          options={EFFORTS.map(value => ({ value, label: value }))}
          onChange={value => set({ effort: value as ReasoningEffort }, "effort")}
        />}
      />
    </SettingsGroup>

    <SettingsGroup label="Custom instructions" note="Used with this agent. Permissions still apply.">
      <SettingsBlockRow>
        <TextArea
          label="Instructions for this agent"
          value={draft.systemPrompt ?? ""}
          placeholder="Describe how you want this agent to work…"
          onChange={value => onDraft({ ...draft, systemPrompt: value })}
        />
      </SettingsBlockRow>
    </SettingsGroup>

    <SaveBar
      dirty={dirty}
      saving={busy}
      canSave={draft.name.trim().length > 0}
      label={isNew ? "New agent" : "Unsaved changes"}
      saveLabel={isNew ? "Create agent" : "Save"}
      onSave={() => void onSave(draft).catch(error => onError(error instanceof Error ? error.message : String(error)))}
      onDiscard={() => (isNew || !stored) ? onBack() : onDraft(structuredClone(stored))}
    />
  </SettingsPage>;
}
