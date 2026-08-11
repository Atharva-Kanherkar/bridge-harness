import { useCallback, useEffect, useId, useRef, useState } from "react";
import { bridgeApi } from "../api";
import type {
  ManagedAgentOperationResult,
  ManagedAgentStatus,
} from "../protocol/generated/protocol";

/**
 * Plug / Play / Remove for the built-in integrations.
 *
 * Holds no installation or ownership logic: every question this panel answers is
 * answered by a field in the `agents` RPC response. In particular removal is
 * offered if and only if `status.removable` is true — the API's own answer to
 * "is this Bridge's to remove". Deriving that from the state string is how a
 * user's own runtime ends up with a Remove button next to it.
 *
 * There is deliberately no progress bar and no cancel control: the RPC runs each
 * operation to completion and exposes neither a progress stream nor a cancel, so
 * an in-flight operation shows an indeterminate busy state. An invented
 * percentage, or a cancel that silently does nothing, would be worse than their
 * absence.
 */

type Busy = { agentId: string; verb: string } | null;

/** How a backing is described to the user, so a user runtime is never presented as Bridge's. */
const SOURCE_LABEL: Record<ManagedAgentStatus["backing"], string> = {
  managed: "Bridge-managed",
  external: "User-managed (on PATH)",
  explicit: "User-managed (custom path)",
  bundled: "Shipped with Bridge",
  none: "Not installed",
};

function stateLabel(status: ManagedAgentStatus): string {
  switch (status.state) {
    case "ready": return "Ready";
    case "installed": return "Installed";
    case "repairable": return "Needs repair";
    case "running": return "Running";
    case "external": return "Available";
    case "not_installed": return "Not installed";
    default: return status.state;
  }
}

export function ManagedAgentsPanel({ initialAgents }: { initialAgents?: ManagedAgentStatus[] }) {
  const [agents, setAgents] = useState<ManagedAgentStatus[] | null>(initialAgents ?? null);
  const [busy, setBusy] = useState<Busy>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [confirming, setConfirming] = useState<ManagedAgentStatus | null>(null);

  useEffect(() => {
    if (initialAgents) return;
    let cancelled = false;
    bridgeApi.listManagedAgents()
      .then(list => { if (!cancelled) setAgents(list.agents); })
      .catch(error => { if (!cancelled) setErrors({ __list: String(error instanceof Error ? error.message : error) }); });
    return () => { cancelled = true; };
  }, [initialAgents]);

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
    setBusy({ agentId: agent.agentId, verb });
    setErrors(current => { const next = { ...current }; delete next[agent.agentId]; return next; });
    try {
      applyResult(await operation(agent.agentId));
    } catch (error) {
      setErrors(current => ({
        ...current,
        [agent.agentId]: error instanceof Error ? error.message : String(error),
      }));
    } finally {
      setBusy(null);
    }
  }, [applyResult]);

  if (errors.__list) {
    return <p role="alert" className="text-sm text-[var(--danger)]">{errors.__list}</p>;
  }
  if (!agents) {
    return <p className="text-sm text-[var(--text-muted)]">Loading agents…</p>;
  }

  return (
    <div className="flex flex-col gap-3">
      {agents.map(agent => (
        <AgentCard
          key={agent.agentId}
          agent={agent}
          busy={busy?.agentId === agent.agentId ? busy.verb : null}
          error={errors[agent.agentId]}
          onInstall={() => run(agent, "Installing", bridgeApi.installManagedAgent)}
          onRepair={() => run(agent, "Repairing", bridgeApi.repairManagedAgent)}
          onRemove={() => setConfirming(agent)}
        />
      ))}
      {confirming && (
        <RemoveConfirmation
          agent={confirming}
          onCancel={() => setConfirming(null)}
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

  return (
    <article
      className="u-glass-soft flex flex-col gap-2 rounded-xl p-4"
      aria-labelledby={`${agent.agentId}-title`}
      data-testid={`agent-card-${agent.agentId}`}
    >
      <header className="flex items-baseline justify-between gap-3">
        <h3 id={`${agent.agentId}-title`} className="text-sm font-medium text-[var(--text)]">
          {agent.label}
        </h3>
        <span className="text-xs text-[var(--text-muted)]" data-testid={`agent-state-${agent.agentId}`}>
          {stateLabel(agent)}
        </span>
      </header>

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs text-[var(--text-muted)]">
        <dt>Source</dt>
        <dd data-testid={`agent-source-${agent.agentId}`}>{SOURCE_LABEL[agent.backing]}</dd>
        {agent.version && (<><dt>Version</dt><dd>{agent.version}</dd></>)}
        {agent.executable && (
          <><dt>Path</dt><dd className="truncate font-mono">{agent.executable}</dd></>
        )}
      </dl>

      {/* Verbatim vendor guidance. Bridge surfaces it and owns none of it: there
          is no field to type a credential into anywhere in this panel. */}
      {agent.vendorMessage && (
        <p className="text-xs text-[var(--warning)]" data-testid={`agent-vendor-${agent.agentId}`}>
          {agent.vendorMessage}
        </p>
      )}

      {error && (
        <p id={errorId} role="alert" className="text-xs text-[var(--danger)]">{error}</p>
      )}

      <div className="flex items-center gap-2">
        {busy ? (
          // Indeterminate on purpose: the RPC reports completion, not progress.
          <span
            role="status"
            aria-live="polite"
            className="text-xs text-[var(--text-muted)] motion-safe:animate-pulse"
            data-testid={`agent-busy-${agent.agentId}`}
          >
            {busy}…
          </span>
        ) : (
          <>
            {absent && (
              <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs" onClick={onInstall}>
                Install
              </button>
            )}
            {needsRepair && (
              <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs" onClick={onRepair}>
                Repair
              </button>
            )}
            {!absent && !needsRepair && agent.backing !== "managed" && (
              <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs" onClick={onInstall}>
                Install Bridge-managed copy
              </button>
            )}

            {/* The one gate on removal: the API said whether this is Bridge's.
                A state string the UI does not recognize can never open it. */}
            {agent.removable && (
              <button
                type="button"
                className="rounded-lg px-3 py-1.5 text-xs text-[var(--danger)] disabled:opacity-50"
                onClick={onRemove}
                disabled={running}
                aria-describedby={running ? `${agent.agentId}-running-reason` : undefined}
              >
                Remove
              </button>
            )}
            {agent.removable && running && (
              <span id={`${agent.agentId}-running-reason`} className="text-xs text-[var(--text-muted)]">
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
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") onCancel(); };
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
      ref={dialog}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      className="u-glass-popover flex flex-col gap-2 rounded-xl p-4"
      data-testid="remove-confirmation"
    >
      <h4 id={titleId} className="text-sm font-medium text-[var(--text)]">
        Remove the Bridge-managed {agent.label}
        {agent.version ? ` ${agent.version}` : ""}?
      </h4>
      {agent.executable && (
        <p className="truncate font-mono text-xs text-[var(--text-muted)]">{agent.executable}</p>
      )}
      <p className="text-xs text-[var(--text-muted)]">
        Your conversation history, vendor configuration, sign-in, and any copy you
        installed yourself are left untouched. You can reinstall at any time.
      </p>
      <div className="flex gap-2">
        <button
          type="button"
          data-autofocus
          className="rounded-lg px-3 py-1.5 text-xs text-[var(--danger)]"
          onClick={onConfirm}
        >
          Remove
        </button>
        <button type="button" className="u-glass rounded-lg px-3 py-1.5 text-xs" onClick={onCancel}>
          Keep it
        </button>
      </div>
    </div>
  );
}
