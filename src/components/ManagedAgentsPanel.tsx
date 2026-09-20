import { ProviderLoginPane } from "./ProviderLoginPane";
import type { UsageProvider } from "../usage";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { bridgeApi } from "../api";
import type {
  ManagedAgentOperationResult,
  ManagedAgentStatus,
} from "../protocol/generated/protocol";
import type { AdapterDescriptor } from "../types";
import { HarnessMark } from "./harnessMarks";
import {
  GhostButton, SettingsGroup, SettingsRow, StatusPill, TextButton, type PillTone,
} from "./settings/kit";

/**
 * Install and remove the built-in agent runtimes.
 *
 * Holds no installation or ownership logic: every question this answers is
 * answered by a field in the `agents` RPC response. In particular removal is
 * offered if and only if `status.removable` is true — the API's own answer to
 * "is this Bridge's to remove". Deriving that from the state string is how a
 * user's own runtime ends up with a Remove button next to it.
 *
 * # Shape
 *
 * The state, the operations, and the confirmation dialog live in
 * `useManagedAgents`, so the Harnesses list and a single harness's detail page
 * read one copy rather than fetching twice and disagreeing about what happened.
 *
 * # What this deliberately does not do
 *
 * **No Start action.** Starting an agent means starting a session with it, which
 * already has a surface: the composer and new-chat flow. A second Start here
 * would either create a session and leave the user sitting in Settings, or need
 * routing this panel has no business owning.
 *
 * **No progress bar and no cancel.** The RPC runs each operation to completion
 * and exposes neither a progress stream nor a cancel, so an in-flight operation
 * shows an indeterminate busy state. An invented percentage, or a cancel that
 * silently does nothing, would be worse than their absence.
 */

/** How a backing is described, so a user runtime is never presented as Bridge's. */
const SOURCE_LABEL: Record<ManagedAgentStatus["backing"], string> = {
  managed: "Bridge-managed",
  external: "Your own install (found on PATH)",
  explicit: "Your own install (configured path)",
  bundled: "Shipped with Bridge",
  none: "Not installed",
};

const STATE_PILL: Record<string, { label: string; tone: PillTone }> = {
  ready: { label: "Ready", tone: "success" },
  installed: { label: "Installed", tone: "success" },
  external: { label: "Working", tone: "success" },
  running: { label: "Running", tone: "info" },
  repairable: { label: "Needs repair", tone: "warning" },
  broken: { label: "Unavailable", tone: "destructive" },
  not_installed: { label: "Not installed", tone: "neutral" },
};

export function stateLabel(status: ManagedAgentStatus): string {
  return STATE_PILL[status.state]?.label ?? status.state;
}

function stateTone(status: ManagedAgentStatus): PillTone {
  return STATE_PILL[status.state]?.tone ?? "neutral";
}

/** Source and version on one line, which is what a row has room for. */
export function sourceLine(agent: ManagedAgentStatus): string {
  return agent.version ? `${SOURCE_LABEL[agent.backing]} · ${agent.version}` : SOURCE_LABEL[agent.backing];
}

export type ManagedAgents = {
  agents: ManagedAgentStatus[] | null;
  listError: string | null;
  busy: Record<string, string>;
  errors: Record<string, string>;
  reload: () => void;
  install: (agent: ManagedAgentStatus) => void;
  repair: (agent: ManagedAgentStatus) => void;
  requestRemove: (agent: ManagedAgentStatus) => void;
  /** Rendered wherever the caller wants; null when nothing is being confirmed. */
  confirmation: ReactNode;
};

export function useManagedAgents(initialAgents?: ManagedAgentStatus[], onChanged?: () => void): ManagedAgents {
  const [agents, setAgents] = useState<ManagedAgentStatus[] | null>(initialAgents ?? null);
  // Keyed by agent id: two runtimes can be working at once, and a single slot
  // let one completion clear another's busy state while its call was still in
  // flight, re-enabling actions mid-operation.
  const [busy, setBusy] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [listError, setListError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<ManagedAgentStatus | null>(null);

  const load = useCallback(() => {
    setListError(null);
    return bridgeApi.listManagedAgents()
      .then(list => setAgents(list.agents))
      .catch(error => setListError(error instanceof Error ? error.message : String(error)));
  }, []);

  useEffect(() => {
    if (initialAgents) return;
    void load();
  }, [initialAgents, load]);

  /** Apply the status the operation returned rather than guessing the new one. */
  const applyResult = useCallback((result: ManagedAgentOperationResult) => {
    setAgents(current =>
      (current ?? []).map(agent => (agent.agentId === result.agentId ? result.status : agent)));
  }, []);

  const run = useCallback(async (
    agent: ManagedAgentStatus,
    verb: string,
    operation: (agentId: string) => Promise<ManagedAgentOperationResult>,
  ) => {
    const agentId = agent.agentId;
    setBusy(current => ({ ...current, [agentId]: verb }));
    setErrors(current => { const next = { ...current }; delete next[agentId]; return next; });
    try {
      applyResult(await operation(agentId));
      onChanged?.();
    } catch (error) {
      setErrors(current => ({
        ...current,
        [agentId]: error instanceof Error ? error.message : String(error),
      }));
    } finally {
      // Only this agent's slot, so a sibling operation still in flight keeps its
      // own busy state.
      setBusy(current => { const next = { ...current }; delete next[agentId]; return next; });
    }
  }, [applyResult, onChanged]);

  const closeConfirm = useCallback(() => setConfirming(null), []);

  return {
    agents,
    listError,
    busy,
    errors,
    reload: () => void load(),
    install: agent => void run(agent, "Installing", bridgeApi.installManagedAgent),
    repair: agent => void run(agent, "Repairing", bridgeApi.repairManagedAgent),
    requestRemove: agent => setConfirming(agent),
    confirmation: confirming
      ? <RemoveConfirmation
          agent={confirming}
          onCancel={closeConfirm}
          onConfirm={() => {
            const agent = confirming;
            setConfirming(null);
            void run(agent, "Removing", bridgeApi.uninstallManagedAgent);
          }}
        />
      : null,
  };
}

/** Whether Bridge has nothing at all for this runtime. */
export function isAbsent(agent: ManagedAgentStatus): boolean {
  return agent.state === "not_installed" && agent.backing === "none";
}

export function agentAuthState(agentId: string, adapters: AdapterDescriptor[]): AdapterDescriptor["authState"] | null {
  return adapters.find(adapter => adapter.id === agentId)?.authState ?? null;
}

function needsRepair(agent: ManagedAgentStatus): boolean {
  return agent.state === "repairable" || agent.state === "broken";
}

/** The action a runtime offers on a list row: Install, Repair, or nothing. */
function RowAction({ agent, state }: { agent: ManagedAgentStatus; state: ManagedAgents }) {
  const busy = state.busy[agent.agentId];
  if (busy) {
    return <span
      role="status"
      aria-live="polite"
      className="shrink-0 text-[11px] text-muted-foreground motion-safe:animate-pulse"
      data-testid={`agent-busy-${agent.agentId}`}
    >{busy}…</span>;
  }
  if (isAbsent(agent)) return <GhostButton onClick={() => state.install(agent)}>Install</GhostButton>;
  if (needsRepair(agent)) return <GhostButton onClick={() => state.repair(agent)}>Repair</GhostButton>;
  return null;
}

/** The Installed / Available groups of the Harnesses list. */
export function ManagedAgentRows({ state, adapters = [], onOpen, onAuthenticationChanged }: {
  state: ManagedAgents;
  adapters?: AdapterDescriptor[];
  onOpen?: (agentId: string) => void;
  onAuthenticationChanged?: () => void;
}) {
  const [login, setLogin] = useState<string | null>(null);
  const groups = useMemo(() => {
    const agents = state.agents ?? [];
    return [
      { label: "Installed", note: "Bridge can start these now", agents: agents.filter(agent => !isAbsent(agent)) },
      { label: "Available", note: "Installed from the vendor's official source", agents: agents.filter(isAbsent) },
    ].filter(group => group.agents.length > 0);
  }, [state.agents]);

  if (state.listError) {
    return <SettingsGroup label="Installed">
      <SettingsRow
        label={<span role="alert" className="text-destructive">{state.listError}</span>}
        control={<GhostButton onClick={state.reload}>Retry</GhostButton>}
      />
    </SettingsGroup>;
  }
  if (!state.agents) {
    return <SettingsGroup label="Installed"><SettingsRow label="Loading agents…" /></SettingsGroup>;
  }

  return <>
    {groups.map(group => <SettingsGroup key={group.label} label={group.label} note={group.note}>
      {group.agents.map(agent => {
        const authState = agentAuthState(agent.agentId, adapters);
        return <div key={agent.agentId} data-testid={`agent-card-${agent.agentId}`}>
        <SettingsRow
          lead={<HarnessMark harness={agent.agentId} size={14} />}
          label={agent.label}
          openLabel={`Configure ${agent.label}`}
          description={<span data-testid={`agent-source-${agent.agentId}`}>{sourceLine(agent)}</span>}
          onOpen={onOpen && (() => onOpen(agent.agentId))}
          control={<>
            <StatusPill tone={stateTone(agent)}>
              <span data-testid={`agent-state-${agent.agentId}`}>{stateLabel(agent)}</span>
            </StatusPill>
            <RowAction agent={agent} state={state} />
            {!isAbsent(agent) && authState === "signed_in" && <span className="text-[11px] font-medium text-success" data-testid={`agent-auth-${agent.agentId}`}>Signed in</span>}
            {!isAbsent(agent) && authState === "signed_out" && <GhostButton onClick={() => setLogin(agent.agentId)}>Sign in</GhostButton>}
            {!isAbsent(agent) && authState === "unknown" && <span className="text-[11px] text-muted-foreground" data-testid={`agent-auth-${agent.agentId}`}>Sign-in status unknown</span>}
          </>}
        />
        {login === agent.agentId && <div className="px-3.5 pb-3"><ProviderLoginPane provider={agent.agentId as UsageProvider} label={agent.label} onClose={() => { setLogin(null); state.reload(); onAuthenticationChanged?.(); }} /></div>}
        {agent.vendorMessage && <p className="px-3.5 pb-2.5 text-[12px] text-warning" data-testid={`agent-vendor-${agent.agentId}`}>
          {agent.vendorMessage}
        </p>}
        {state.errors[agent.agentId] && <p role="alert" className="px-3.5 pb-2.5 text-[12px] text-destructive">
          {state.errors[agent.agentId]}
        </p>}
      </div>})}
    </SettingsGroup>)}
  </>;
}

/**
 * The runtime block of a harness detail page: what Bridge has, and what it can
 * do about it. Remove appears here and nowhere else, because a destructive
 * action belongs on the page about the thing, not in a list of nine.
 */
export function ManagedAgentDetail({ state, agentId }: { state: ManagedAgents; agentId: string }) {
  const reasonId = useId();
  const agent = state.agents?.find(item => item.agentId === agentId);
  if (!agent) return null;
  const busy = state.busy[agentId];
  const running = agent.state === "running";

  return <SettingsGroup label="Runtime" note={<StatusPill tone={stateTone(agent)}>
    <span data-testid={`agent-state-${agent.agentId}`}>{stateLabel(agent)}</span>
  </StatusPill>}>
    <SettingsRow
      lead={<HarnessMark harness={agent.agentId} size={14} />}
      label={agent.label}
      description={<span data-testid={`agent-source-${agent.agentId}`}>{sourceLine(agent)}</span>}
      control={busy
        ? <span role="status" aria-live="polite" className="text-[11px] text-muted-foreground motion-safe:animate-pulse" data-testid={`agent-busy-${agent.agentId}`}>{busy}…</span>
        : <>
            {isAbsent(agent) && <GhostButton onClick={() => state.install(agent)}>Install</GhostButton>}
            {needsRepair(agent) && <GhostButton onClick={() => state.repair(agent)}>Repair</GhostButton>}
            {/* Deliberately quiet: the agent already works, so a managed copy is
                an opt-in, not a call to action. Making it the only button read
                as "this needs installing" for an agent the user can already
                chat with. */}
            {!isAbsent(agent) && !needsRepair(agent) && agent.backing !== "managed" && <TextButton onClick={() => state.install(agent)}>
              Let Bridge manage its own copy
            </TextButton>}
            {/* The one gate on removal: the API said whether this is Bridge's.
                A state string the UI does not recognize can never open it. */}
            {agent.removable && <TextButton
              tone="destructive"
              disabled={running}
              onClick={() => state.requestRemove(agent)}
            >Remove</TextButton>}
          </>}
    />
    {agent.executable && <SettingsRow label="Path" description={agent.executable} mono />}
    {!isAbsent(agent) && !needsRepair(agent) && agent.backing !== "managed" && <SettingsRow
      label="Nothing to install"
      description="Bridge uses this copy. A managed copy is a separate download Bridge can update and remove on its own; yours stays where it is."
    />}
    {agent.removable && running && <SettingsRow label={<span id={reasonId}>Stop {agent.label} before removing it</span>} />}
    {/* Verbatim vendor guidance. Bridge surfaces it and owns none of it: there
        is no field to type a credential into anywhere here. */}
    {agent.vendorMessage && <SettingsRow label={<span className="text-warning" data-testid={`agent-vendor-${agent.agentId}`}>{agent.vendorMessage}</span>} />}
    {state.errors[agentId] && <SettingsRow label={<span role="alert" className="text-destructive">{state.errors[agentId]}</span>} />}
  </SettingsGroup>;
}

/** The list on its own, for callers that want the runtimes and nothing else. */
export function ManagedAgentsPanel({ initialAgents, adapters, onOpen, onChanged }: {
  initialAgents?: ManagedAgentStatus[];
  adapters?: AdapterDescriptor[];
  onOpen?: (agentId: string) => void;
  onChanged?: () => void;
}) {
  const state = useManagedAgents(initialAgents, onChanged);
  return <div className="space-y-[26px]">
    <ManagedAgentRows state={state} adapters={adapters} onOpen={onOpen} onAuthenticationChanged={onChanged} />
    {state.confirmation}
  </div>;
}

/**
 * Names the exact payload being removed.
 *
 * A destructive confirmation that says "remove this" tells the user nothing, so
 * this one carries the label, the version, and the path that will be deleted, and
 * states plainly what is left alone.
 *
 * Focus lands on **Keep it**, not Remove: a dialog that autofocuses its
 * destructive action turns a stray Enter into an uninstall. Keep it also comes
 * first in tab order for the same reason.
 */
function RemoveConfirmation({ agent, onCancel, onConfirm }: {
  agent: ManagedAgentStatus;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const titleId = useId();
  const dialog = useRef<HTMLDivElement | null>(null);
  const opener = useRef<Element | null>(null);

  useEffect(() => {
    opener.current = document.activeElement;
    dialog.current?.querySelector<HTMLButtonElement>("[data-autofocus]")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { onCancel(); return; }
      if (event.key !== "Tab") return;
      // A real trap, since this claims aria-modal: Tab cycles within the dialog
      // instead of wandering into the row actions behind the overlay.
      const focusable = [...(dialog.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!event.shiftKey && active === last) { event.preventDefault(); first.focus(); }
      if (event.shiftKey && active === first) { event.preventDefault(); last.focus(); }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      // Focus goes back where it came from, so a keyboard user is not dropped at
      // the top of the document.
      (opener.current as HTMLElement | null)?.focus?.();
    };
  }, [onCancel]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-scrim p-4 pt-[8vh] sm:pt-[10vh]"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-testid="remove-confirmation"
    >
      <div ref={dialog} className="u-glass-popover flex max-h-[84dvh] w-full max-w-md flex-col gap-2 overflow-y-auto rounded-xl p-4">
        <h4 id={titleId} className="text-[13px] font-medium text-foreground">
          Remove the Bridge-managed {agent.label}
          {agent.version ? ` ${agent.version}` : ""}?
        </h4>
        {agent.executable && (
          <p className="truncate font-mono text-[11px] text-muted-foreground" title={agent.executable}>
            {agent.executable}
          </p>
        )}
        <p className="text-[12px] text-muted-foreground">
          Your conversation history, vendor configuration, sign-in, and any copy you
          installed yourself are left untouched. You can reinstall at any time.
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            data-autofocus
            className="inline-flex h-7 items-center rounded-lg border border-border-card bg-popover px-2.5 text-xs text-foreground transition-colors hover:bg-accent"
            onClick={onCancel}
          >
            Keep it
          </button>
          <TextButton tone="destructive" onClick={onConfirm}>Remove</TextButton>
        </div>
      </div>
    </div>
  );
}
