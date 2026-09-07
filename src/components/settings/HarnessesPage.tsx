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

import { useMemo } from "react";
import type { HarnessConfig, OpenCodeCatalog, ReasoningEffort } from "../../types";
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
};

export function HarnessesPage(props: Shared & {
  detailId: string | null;
  onOpenDetail: (id: string) => void;
  onCloseDetail: () => void;
}) {
  const { harnesses, managed, detailId, onOpenDetail, onCloseDetail } = props;
  const detail = detailId ? harnesses.find(item => item.id === detailId) : undefined;
  if (detail) return <HarnessDetail {...props} harness={detail} onBack={onCloseDetail} />;

  const bridge = harnesses.find(item => item.id === "bridge");
  return <SettingsPage
    title="Harnesses"
    description="The runtimes Bridge can start. Defaults apply to new sessions; provider credentials stay in each harness's own store."
  >
    <ManagedAgentRows state={managed} onOpen={onOpenDetail} />
    {bridge && <SettingsGroup label="Defaults" note="Bridge's own routing">
      <SettingsRow
        lead={<HarnessMark harness="bridge" size={14} />}
        label={bridge.label}
        openLabel="Configure Bridge"
        description="Chooses a runtime for you when a session does not name one"
        onOpen={() => onOpenDetail("bridge")}
        control={<StatusPill tone="success">Always on</StatusPill>}
      />
    </SettingsGroup>}
    {managed.confirmation}
  </SettingsPage>;
}

function HarnessDetail({
  harness, modelOptions, managed, busy, openCodeCatalog, openCodeDiscoveryError,
  drafts, onDraft, onDiscard, onSaveHarness, onResetHarness, onCatalog, onError, onBack,
}: Shared & { harness: HarnessConfig; onBack: () => void }) {
  const [isFlashed, flash] = useSavedFlash();
  const draft = drafts[harness.id] ?? {};
  const isOpenCode = harness.id === "opencode";
  const isBridge = harness.id === "bridge";
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
    breadcrumb={[{ label: "Harnesses", onClick: onBack }, { label: harness.label }]}
    description={harness.isOverride ? "Customized" : "Bridge defaults"}
    action={harness.isOverride
      ? <TextButton disabled={busy} onClick={() => void onResetHarness(harness.id)}>Reset to defaults</TextButton>
      : undefined}
  >
    {!isBridge && <ManagedAgentDetail state={managed} agentId={harness.id} />}

    <SettingsGroup label="Sessions">
      <SettingsRow
        label="Enabled"
        description="New sessions may use this harness."
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
          onChange={value => void persist({ defaultModel: value || null }, "model")}
        />}
      />
      <SettingsRow
        label="Default effort"
        saved={isFlashed("effort")}
        control={<Select
          label={`${harness.label} default effort`}
          value={harness.effort ?? ""}
          disabled={busy}
          width="w-40"
          options={[{ value: "", label: "Role default" }, ...EFFORTS.map(value => ({ value, label: value }))]}
          onChange={value => void persist({ effort: (value || null) as ReasoningEffort | null }, "effort")}
        />}
      />
    </SettingsGroup>

    <SettingsGroup label="System prompt" note="Appended after Bridge safety and routing policy">
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
        : <SettingsBlockRow label="Advanced JSON" description="Harness options Bridge has no row for yet.">
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
