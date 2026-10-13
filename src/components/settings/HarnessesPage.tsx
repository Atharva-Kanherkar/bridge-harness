// Harnesses: a list of runtimes, and one detail page each.
//
// The old page was a 3800px scroll that mixed two different questions: whether a
// runtime is installed at all (`ManagedAgentsPanel`) and how it behaves once it
// runs (the harness config form). They are now the list and the detail of the
// same object, so the first question is answered by a row and the second by
// opening it.
//
// Saving follows the screen's one rule. Enabled, Default model, Default effort,
// and the visible-model switches persist on change, and each writes the *stored*
// record with exactly one field replaced, so a half-typed system prompt is never
// dragged along by a switch. The system prompt, the executable path, and the
// advanced JSON are text, so they dirty the page and go through the save bar.

import { useMemo, type ReactNode } from "react";
import type { AdapterDescriptor, HarnessConfig, OpenCodeCatalog, ReasoningEffort } from "../../types";
import { advertisedEfforts } from "../../modelProfiles";
import { HarnessMark } from "../harnessMarks";
import { ManagedAgentDetail, ManagedAgentRows, type ManagedAgents } from "../ManagedAgentsPanel";
import { OpenCodeHarnessSettings, type OpenCodeAdvancedSettings } from "../OpenCodeHarnessSettings";
import {
  Field, SaveBar, Select, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow, StatusPill,
  Switch, TextArea, TextButton, useSavedFlash,
} from "./kit";

const EFFORTS: ReasoningEffort[] = ["low", "medium", "high", "xhigh"];

/** Unsaved text for one harness. Absent keys are clean. */
export type HarnessDraft = {
  systemPrompt?: string;
  advancedText?: string;
  executablePath?: string;
};

export type ModelOption = { adapter: string; id: string; label: string };

type Shared = {
  adapters: AdapterDescriptor[];
  harnesses: HarnessConfig[];
  modelOptions: ModelOption[];
  managed: ManagedAgents;
  busy: boolean;
  openCodeCatalog?: OpenCodeCatalog;
  openCodeDiscoveryError?: string;
  drafts: Record<string, HarnessDraft>;
  onDraft: (id: string, patch: HarnessDraft) => void;
  onDiscard: (id: string) => void;
  onSaveHarness: (next: HarnessConfig) => Promise<void>;
  onResetHarness: (id: string) => Promise<void>;
  onCatalog: (catalog: OpenCodeCatalog) => void;
  onError: (message: string) => void;
  onAuthenticationChanged: () => void;
};

export function HarnessesPage(props: Shared & {
  detailId: string | null;
  onOpenDetail: (id: string) => void;
  onCloseDetail: () => void;
  preferences?: ReactNode;
}) {
  const { harnesses, managed, adapters, detailId, onOpenDetail, onCloseDetail, onAuthenticationChanged } = props;
  const detail = detailId ? harnesses.find(item => item.id === detailId) : undefined;
  if (detail) return <HarnessDetail {...props} harness={detail} onBack={onCloseDetail} />;

  const bridge = harnesses.find(item => item.id === "bridge");
  return <SettingsPage
    title="Coding agents"
    description="Use coding agents already on your computer, or let Bridge install a separate copy. Open an agent to manage sign-in and defaults for new chats."
  >
    <ManagedAgentRows state={managed} adapters={adapters} onOpen={onOpenDetail} onAuthenticationChanged={onAuthenticationChanged} />
    {harnesses.filter(harness => harness.id === "pi").map(harness => {
      const adapter = adapters.find(adapter => adapter.id === harness.id);
      return <SettingsGroup key={harness.id} label="Pi">
        <SettingsRow lead={<HarnessMark harness="pi" size={14} />} label="Pi" openLabel="Configure Pi"
          description={adapter?.available ? "Models and sign-in managed in Pi" : adapter?.unavailableReason ?? "Checking Pi availability"}
          onOpen={() => onOpenDetail(harness.id)}
          control={<StatusPill tone={adapter?.available ? "success" : "neutral"}>{adapter?.available ? "Available" : "Unavailable"}</StatusPill>} />
      </SettingsGroup>;
    })}
    {props.preferences}
    {bridge && <details className="text-ui"><summary className="cursor-pointer text-muted-foreground">Advanced Bridge defaults</summary><div className="mt-3"><SettingsGroup label="Defaults" note="Bridge's own routing">
      <SettingsRow
        lead={<HarnessMark harness="bridge" size={14} />}
        label={bridge.label}
        openLabel="Configure Bridge"
        description="Chooses a runtime for you when a session does not name one"
        onOpen={() => onOpenDetail("bridge")}
        control={<StatusPill tone="success">Always on</StatusPill>}
      />
    </SettingsGroup></div></details>}
    {managed.confirmation}
  </SettingsPage>;
}

function HarnessDetail({
  harness, adapters, onAuthenticationChanged, modelOptions, managed, busy, openCodeCatalog, openCodeDiscoveryError,
  drafts, onDraft, onDiscard, onSaveHarness, onResetHarness, onCatalog, onError, onBack,
}: Shared & { harness: HarnessConfig; onBack: () => void }) {
  const [isFlashed, flash] = useSavedFlash();
  const draft = drafts[harness.id] ?? {};
  const isOpenCode = harness.id === "opencode";
  const isBridge = harness.id === "bridge";
  const isPi = harness.id === "pi";
  const descriptor = adapters.find(adapter => adapter.id === harness.id);
  const selectedModel = descriptor?.models.find(model => model.id === (harness.defaultModel ?? descriptor.defaultModel));
  const efforts = isPi ? advertisedEfforts(selectedModel) : EFFORTS;
  const advanced = (harness.advanced ?? {}) as OpenCodeAdvancedSettings;
  const storedAdvancedText = JSON.stringify(harness.advanced ?? {}, null, 2);
  const dirty = draft.systemPrompt !== undefined || draft.advancedText !== undefined || draft.executablePath !== undefined;

  const options = useMemo(
    () => modelOptions.filter(option => option.adapter === harness.id).map(option => ({ value: option.id, label: option.label })),
    [modelOptions, harness.id],
  );

  /** Write the stored record with exactly one field replaced. */
  const persist = async (patch: Partial<HarnessConfig>, key: string) => {
    try { await onSaveHarness({ ...harness, ...patch }); flash(key); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
  };

  const save = async () => {
    let nextAdvanced: Record<string, unknown> = { ...advanced } as Record<string, unknown>;
    if (draft.advancedText !== undefined) {
      try {
        const parsed: unknown = JSON.parse(draft.advancedText || "{}");
        if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") throw new Error("Advanced configuration must be a JSON object");
        nextAdvanced = parsed as Record<string, unknown>;
      } catch (error) {
        onError(error instanceof Error ? error.message : String(error));
        return;
      }
    }
    if (draft.executablePath !== undefined) {
      nextAdvanced = { ...nextAdvanced, executablePath: draft.executablePath.trim() || undefined };
    }
    try {
      await onSaveHarness({
        ...harness,
        systemPrompt: draft.systemPrompt ?? harness.systemPrompt ?? "",
        advanced: nextAdvanced,
      });
      onDiscard(harness.id);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  return <SettingsPage
    title={harness.label}
    breadcrumb={[{ label: "Coding agents", onClick: onBack }, { label: harness.label }]}
    description={`Defaults and instructions for new ${harness.label} chats. Chat-specific choices take priority.`}
    action={harness.isOverride
      ? <TextButton disabled={busy} onClick={() => void onResetHarness(harness.id)}>Reset to defaults</TextButton>
      : undefined}
  >
    {!isBridge && <ManagedAgentDetail state={managed} agentId={harness.id} adapters={adapters} onAuthenticationChanged={onAuthenticationChanged} />}

    {isPi && <SettingsGroup label="Runtime" note={<StatusPill tone={descriptor?.available ? "success" : "neutral"}>{descriptor?.available ? `Pi ${descriptor.version ?? ""}` : "Unavailable"}</StatusPill>}>
      <SettingsRow label="Provider setup" description="Install @earendil-works/pi-coding-agent, then configure local or hosted models and sign-in with pi."
        control={<TextButton disabled={busy} onClick={onAuthenticationChanged}>Refresh Pi</TextButton>} />
      {descriptor?.unavailableReason && <SettingsBlockRow><p role="status" className="text-ui text-muted-foreground">{descriptor.unavailableReason}</p></SettingsBlockRow>}
      <SettingsBlockRow><p className="text-ui text-muted-foreground">Pi runs with full access and asks Bridge to approve each tool. Scoped sandboxes, project extensions, and hidden memory or credential context are unavailable.</p></SettingsBlockRow>
    </SettingsGroup>}
    <SettingsGroup label="New chats">
      <SettingsRow
        label="Enabled"
        description="Allow Bridge to use this coding agent."
        saved={isFlashed("enabled")}
        control={<Switch
          label={`${harness.label} enabled`}
          checked={harness.enabled}
          disabled={busy}
          onChange={next => void persist({ enabled: next }, "enabled")}
        />}
      />
      <SettingsRow
        label="Default model"
        saved={isFlashed("model")}
        control={<Select
          label={`${harness.label} default model`}
          value={harness.defaultModel ?? ""}
          disabled={busy || isBridge}
          options={[{ value: "", label: "Automatic" }, ...options]}
          onChange={value => {
            const nextModel = descriptor?.models.find(model => model.id === (value || descriptor.defaultModel));
            const effort = isPi && harness.effort && !advertisedEfforts(nextModel).includes(harness.effort) ? null : harness.effort;
            void persist({ defaultModel: value || null, ...(isPi ? { effort } : {}) }, "model");
          }}
        />}
      />
      <SettingsRow
        description={isPi && efforts.length === 0 ? "Pi uses this model’s own thinking setting." : undefined}
        label="Thinking level"
        saved={isFlashed("effort")}
        control={<Select
          label={`${harness.label} default effort`}
          value={harness.effort ?? ""}
          disabled={busy || (isPi && efforts.length === 0)}
          width="w-40"
          options={[{ value: "", label: "Role default" }, ...efforts.map(value => ({ value, label: value }))]}
          onChange={value => void persist({ effort: (value || null) as ReasoningEffort | null }, "effort")}
        />}
      />
    </SettingsGroup>

    <SettingsGroup label="Custom instructions" note="Used in new chats with this agent. Permissions still apply.">
      <SettingsBlockRow>
        <TextArea
          label={`${harness.label} system prompt`}
          value={draft.systemPrompt ?? harness.systemPrompt ?? ""}
          placeholder={`Instructions for every ${harness.label} session…`}
          onChange={value => onDraft(harness.id, { systemPrompt: value })}
        />
      </SettingsBlockRow>
    </SettingsGroup>

    {isOpenCode && <OpenCodeHarnessSettings
      value={advanced}
      catalog={openCodeCatalog}
      discoveryError={openCodeDiscoveryError}
      disabled={busy}
      onCatalog={onCatalog}
      onError={onError}
      onChange={value => {
        // A model that just went invisible cannot stay the default.
        const stillVisible = !harness.defaultModel
          || !value.visibleModels?.length
          || value.visibleModels.includes(harness.defaultModel);
        void persist({
          advanced: { ...advanced, ...value },
          defaultModel: stillVisible ? harness.defaultModel : null,
        }, "visible");
      }}
    />}

    {!isBridge && <SettingsGroup label="Advanced">
      {isOpenCode
        ? <SettingsRow
            label="Executable path"
            description="Optional. The managed executable and PATH are the fallback locations."
            control={<Field
              label="OpenCode executable path"
              value={draft.executablePath ?? advanced.executablePath ?? ""}
              placeholder="/path/to/opencode"
              mono
              onChange={value => onDraft(harness.id, { executablePath: value })}
            />}
          />
        : <SettingsBlockRow label="Advanced configuration (JSON)" description="Agent-specific options. Edit only if you know which option your agent needs.">
            <TextArea
              label={`${harness.label} advanced JSON`}
              rows={5}
              value={draft.advancedText ?? storedAdvancedText}
              onChange={value => onDraft(harness.id, { advancedText: value })}
            />
          </SettingsBlockRow>}
    </SettingsGroup>}

    <SaveBar dirty={dirty} saving={busy} onSave={() => void save()} onDiscard={() => onDiscard(harness.id)} />
    {managed.confirmation}
  </SettingsPage>;
}
