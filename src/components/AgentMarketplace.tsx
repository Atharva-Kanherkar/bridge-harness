import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, LoaderCircle, RefreshCw } from "lucide-react";
import { bridgeApi } from "../api";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";

/** Agents tab: a card per agent, one button each — Install or Uninstall. */

const BLURB: Record<string, string> = {
  claude: "Anthropic's coding agent",
  codex: "OpenAI's coding agent",
  opencode: "Open-source coding agent",
};

export function AgentMarketplace() {
  const [agents, setAgents] = useState<ManagedAgentStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<Record<string, string>>({});
  const [failures, setFailures] = useState<Record<string, string>>({});

  // Bumped every time an operation writes a row. A refresh that started before
  // that write is discarding a list it read *before* the install landed, so
  // applying it would flip the card back to Install.
  const writes = useRef(0);

  const refresh = useCallback(async () => {
    setLoading(true);
    const before = writes.current;
    try {
      const list = await bridgeApi.listManagedAgents();
      // A newer result already won. The list in hand is stale by construction.
      if (writes.current !== before) return;
      setAgents(list.agents);
      setError(null);
    } catch (failure) {
      if (writes.current !== before) return;
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const working = Object.keys(busy).length > 0;

  const run = useCallback(async (
    id: string,
    verb: string,
    operation: (agentId: string) => Promise<{ agentId: string; status: ManagedAgentStatus }>,
  ) => {
    setBusy(current => ({ ...current, [id]: verb }));
    setFailures(current => { const next = { ...current }; delete next[id]; return next; });
    try {
      const result = await operation(id);
      writes.current += 1;
      setAgents(current => (current ?? []).map(a => (a.agentId === result.agentId ? result.status : a)));
    } catch (failure) {
      setFailures(current => ({ ...current, [id]: failure instanceof Error ? failure.message : String(failure) }));
    } finally {
      setBusy(current => { const next = { ...current }; delete next[id]; return next; });
    }
  }, []);

  return <div className="h-full min-h-0 overflow-y-auto">
    <main className="mx-auto w-full max-w-5xl px-5 pb-16 pt-8 sm:px-8 sm:pt-12">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="font-display text-[32px] font-semibold tracking-[-0.025em] text-white">Agents</h1>
          <p className="mt-1.5 text-[13.5px] leading-relaxed text-neutral-500">Install and uninstall coding agents</p>
        </div>
        <button type="button" onClick={() => void refresh()} disabled={loading || working} aria-label="Refresh agents"
          className="mt-1.5 inline-flex h-8 w-8 items-center justify-center rounded-full text-neutral-500 transition-colors hover:bg-white/[0.06] hover:text-neutral-200 disabled:opacity-40">
          <RefreshCw size={14} className={loading ? "animate-spin" : ""}/>
        </button>
      </div>

      {error && <div role="alert" className="mt-4 flex items-start gap-2 rounded-xl border border-red-400/15 bg-red-400/[0.04] px-3 py-2 text-[10.5px] text-red-200/80">
        <AlertCircle className="mt-0.5 shrink-0" size={12}/><span>{error}</span>
        <button type="button" onClick={() => void refresh()} className="ml-auto shrink-0 underline underline-offset-2">Retry</button>
      </div>}

      {loading && !agents && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-neutral-500">
        <LoaderCircle className="animate-spin" size={15}/> Loading agents…
      </div>}

      {agents && <div className="mt-6 grid grid-cols-1 gap-3 lg:grid-cols-2">
        {agents.map(agent => <AgentCard
          key={agent.agentId}
          agent={agent}
          busy={busy[agent.agentId] ?? null}
          error={failures[agent.agentId]}
          onInstall={() => void run(agent.agentId, "Installing", bridgeApi.installManagedAgent)}
          onUninstall={() => void run(agent.agentId, "Uninstalling", bridgeApi.uninstallManagedAgent)}
        />)}
      </div>}
    </main>
  </div>;
}

function AgentCard({ agent, busy, error, onInstall, onUninstall }: {
  agent: ManagedAgentStatus;
  busy: string | null;
  error?: string;
  onInstall: () => void;
  onUninstall: () => void;
}) {
  // Uninstall only where Bridge owns the copy — the API's own `removable`.
  // Anywhere else the call would fail, and on a PATH install it would mean
  // deleting something the user put there.
  const installed = agent.removable;

  return <article className="u-glass-soft flex items-center gap-3.5 rounded-2xl px-4 py-3.5" data-testid={`agent-card-${agent.agentId}`}>
    <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-[13px] border border-white/[0.09] bg-gradient-to-br from-white/[0.08] to-white/[0.02] font-display text-[13px] font-semibold text-neutral-200">
      {agent.label.slice(0, 2).toUpperCase()}
    </div>

    <div className="min-w-0 flex-1">
      <h3 className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-neutral-100">{agent.label}</h3>
      <p className="mt-0.5 truncate text-[11.5px] text-neutral-500">
        {BLURB[agent.agentId] ?? "Coding agent"}{agent.version ? ` · ${agent.version}` : ""}
      </p>
      {error && <p role="alert" className="mt-1 text-[10.5px] text-red-400">{error}</p>}
    </div>

    {busy
      ? <span role="status" aria-live="polite" data-testid={`agent-busy-${agent.agentId}`}
          className="shrink-0 text-[11px] text-neutral-500 motion-safe:animate-pulse">{busy}…</span>
      : installed
        ? <button type="button" onClick={onUninstall}
            className="shrink-0 rounded-full border border-white/[0.12] px-4 py-1.5 text-[11.5px] font-medium text-neutral-300 transition-colors hover:bg-white/[0.06]">
            Uninstall
          </button>
        : <button type="button" onClick={onInstall}
            className="shrink-0 rounded-full bg-white px-4 py-1.5 text-[11.5px] font-medium text-black transition-opacity hover:opacity-90">
            Install
          </button>}
  </article>;
}
