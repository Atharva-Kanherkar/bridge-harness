import { useCallback, useEffect, useId, useRef, useState } from "react";
import { bridgeApi } from "../api";
import type {
  ManagedAgentOperationResult,
  ManagedAgentStatus,
} from "../protocol/generated/protocol";

/**
 * Install and remove the built-in agent runtimes.
 *
 * Holds no installation or ownership logic: every question this panel answers is
 * answered by a field in the `agents` RPC response. In particular removal is
 * offered if and only if `status.removable` is true — the API's own answer to
 * "is this Bridge's to remove". Deriving that from the state string is how a
 * user's own runtime ends up with a Remove button next to it.
 *
 * # What this panel deliberately does not do
 *
 * **No Start action.** Starting an agent means starting a session with it, which
 * already has a surface: the composer and new-chat flow. A second Start here
 * would either create a session and leave the user sitting in Settings, or need
 * routing this panel has no business owning. Issue #170's acceptance says Start
 * happens "through the unchanged integration", which is that existing flow.
 *
 * **No progress bar and no cancel.** The RPC runs each operation to completion
 * and exposes neither a progress stream nor a cancel, so an in-flight operation
 * shows an indeterminate busy state. An invented percentage, or a cancel that
 * silently does nothing, would be worse than their absence. Both arrive with the
 * background job, which is a new result shape rather than a UI change.
 */

/** How a backing is described, so a user runtime is never presented as Bridge's. */
const SOURCE_LABEL: Record<ManagedAgentStatus["backing"], string> = {
  managed: "Bridge-managed",
  external: "Your own install (found on PATH)",
  explicit: "Your own install (configured path)",
  bundled: "Shipped with Bridge",
  none: "Not installed",
};

function stateLabel(status: ManagedAgentStatus): string {
  switch (status.state) {
    case "ready": return "Ready";
    case "installed": return "Installed";
    case "repairable": return "Needs repair";
    case "broken": return "Unavailable";
    case "running": return "Running";
    case "external": return "Working";
    case "not_installed": return "Not installed";
    default: return status.state;
  }
}

export function ManagedAgentsPanel({ initialAgents }: { initialAgents?: ManagedAgentStatus[] }) {
  const [agents, setAgents] = useState<ManagedAgentStatus[] | null>(initialAgents ?? null);
  // Keyed by agent id: two cards can be working at once, and a single slot let
  // one card's completion clear another's busy state while its call was still
  // in flight, re-enabling actions mid-operation.
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
  }, [applyResult]);

  const closeConfirm = useCallback(() => setConfirming(null), []);

  if (listError) {
    return (
      <div className="flex items-center gap-3">
        <p role="alert" className="text-sm text-red-400">{listError}</p>
        <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs" onClick={() => void load()}>
          Retry
        </button>
      </div>
    );
  }
  if (!agents) {
    return <p className="text-sm text-muted-foreground">Loading agents…</p>;
  }

  return (
    <div className="flex flex-col gap-3">
      {agents.map(agent => (
        <AgentCard
          key={agent.agentId}
          agent={agent}
          busy={busy[agent.agentId] ?? null}
          error={errors[agent.agentId]}
          onInstall={() => void run(agent, "Installing", bridgeApi.installManagedAgent)}
          onRepair={() => void run(agent, "Repairing", bridgeApi.repairManagedAgent)}
          onRemove={() => setConfirming(agent)}
        />
      ))}
      {confirming && (
        <RemoveConfirmation
          agent={confirming}
          onCancel={closeConfirm}
          onConfirm={() => {
            const agent = confirming;
            setConfirming(null);
            void run(agent, "Removing", bridgeApi.uninstallManagedAgent);
          }}
        />
      )}
    </div>
  );
}

function AgentCard({ agent, busy, error, onInstall, onRepair, onRemove }: {
  agent: ManagedAgentStatus;
  busy: string | null;
  error?: string;
  onInstall: () => void;
  onRepair: () => void;
  onRemove: () => void;
}) {
  const running = agent.state === "running";
  const needsRepair = agent.state === "repairable" || agent.state === "broken";
  const absent = agent.state === "not_installed" && agent.backing === "none";
  const errorId = useId();
  const reasonId = useId();
  // Every action carries the card's error, so a screen reader reaches the reason
  // from the control that failed rather than having to hunt for it.
  const describedBy = [error ? errorId : null].filter(Boolean).join(" ") || undefined;

  return (
    <article
      className="u-glass-soft flex flex-col gap-2 rounded-xl p-4"
      aria-labelledby={`${agent.agentId}-title`}
      data-testid={`agent-card-${agent.agentId}`}
    >
      <header className="flex items-baseline justify-between gap-3">
        {/* h4: nested under the section's "Agent runtimes" h3. */}
        <h4 id={`${agent.agentId}-title`} className="text-sm font-medium text-foreground">
          {agent.label}
        </h4>
        <span className="text-xs text-muted-foreground" data-testid={`agent-state-${agent.agentId}`}>
          {stateLabel(agent)}
        </span>
      </header>

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <dt>Source</dt>
        <dd data-testid={`agent-source-${agent.agentId}`}>{SOURCE_LABEL[agent.backing]}</dd>
        {agent.version && (<><dt>Version</dt><dd>{agent.version}</dd></>)}
        {agent.executable && (
          <>
            <dt>Path</dt>
            {/* title, because the path is truncated and it is the thing a user
                needs to read to know which copy this is. */}
            <dd className="truncate font-mono" title={agent.executable}>{agent.executable}</dd>
          </>
        )}
      </dl>

      {/* Verbatim vendor guidance. Bridge surfaces it and owns none of it: there
          is no field to type a credential into anywhere in this panel. */}
      {agent.vendorMessage && (
        <p className="text-xs text-amber-300" data-testid={`agent-vendor-${agent.agentId}`}>
          {agent.vendorMessage}
        </p>
      )}

      {error && <p id={errorId} role="alert" className="text-xs text-red-400">{error}</p>}

      <div className="flex items-center gap-2">
        {busy ? (
          // Indeterminate on purpose: the RPC reports completion, not progress.
          <span
            role="status"
            aria-live="polite"
            className="text-xs text-muted-foreground motion-safe:animate-pulse"
            data-testid={`agent-busy-${agent.agentId}`}
          >
            {busy}…
          </span>
        ) : (
          <>
            {absent && (
              <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs"
                onClick={onInstall} aria-describedby={describedBy}>
                Install
              </button>
            )}
            {needsRepair && (
              <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs"
                onClick={onRepair} aria-describedby={describedBy}>
                Repair
              </button>
            )}
            {!absent && !needsRepair && agent.backing !== "managed" && (
              <>
                <span className="text-xs text-muted-foreground">
                  Bridge uses this copy. Nothing to install.
                </span>
                {/* Deliberately quiet: the agent already works, so this is an
                    opt-in, not a call to action. #170 calls it optional, and
                    making it the only button on the card read as "this needs
                    installing" for an agent the user can already chat with. */}
                <button
                  type="button"
                  className="ml-auto text-xs text-muted-foreground underline decoration-dotted underline-offset-2 hover:text-foreground"
                  onClick={onInstall}
                  aria-describedby={describedBy}
                  title="Downloads a separate copy that Bridge can update and remove on its own. Your install stays where it is."
                >
                  Let Bridge manage its own copy
                </button>
              </>
            )}

            {/* The one gate on removal: the API said whether this is Bridge's.
                A state string the UI does not recognize can never open it. */}
            {agent.removable && (
              <button
                type="button"
                className="rounded-lg px-3 py-1.5 text-xs text-red-400 hover:text-red-300 disabled:opacity-50"
                onClick={onRemove}
                disabled={running}
                aria-describedby={[running ? reasonId : null, error ? errorId : null]
                  .filter(Boolean).join(" ") || undefined}
              >
                Remove
              </button>
            )}
            {agent.removable && running && (
              <span id={reasonId} className="text-xs text-muted-foreground">
                Stop {agent.label} before removing it
              </span>
            )}
          </>
        )}
      </div>
    </article>
  );
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
      // instead of wandering into the card actions behind the overlay.
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
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/45 p-4 pt-[10vh] backdrop-blur-md"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-testid="remove-confirmation"
    >
      <div ref={dialog} className="u-glass-popover flex w-full max-w-md flex-col gap-2 rounded-xl p-4">
        <h4 id={titleId} className="text-sm font-medium text-foreground">
          Remove the Bridge-managed {agent.label}
          {agent.version ? ` ${agent.version}` : ""}?
        </h4>
        {agent.executable && (
          <p className="truncate font-mono text-xs text-muted-foreground" title={agent.executable}>
            {agent.executable}
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          Your conversation history, vendor configuration, sign-in, and any copy you
          installed yourself are left untouched. You can reinstall at any time.
        </p>
        <div className="flex gap-2">
          <button
            type="button"
            data-autofocus
            className="u-glass rounded-lg px-3 py-1.5 text-xs"
            onClick={onCancel}
          >
            Keep it
          </button>
          <button
            type="button"
            className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-1.5 text-xs text-red-300"
            onClick={onConfirm}
          >
            Remove
          </button>
        </div>
      </div>
    </div>
  );
}
