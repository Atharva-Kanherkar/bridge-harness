import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
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
    <div className={SCREEN_CONTENT}>
      <ScreenHeading title="Agents" description="Install and manage coding agents." action={<button type="button" onClick={() => void refresh()} disabled={loading || working} aria-label="Refresh agents"
          className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
          <RefreshCw size={14} className={loading ? "animate-spin" : ""}/>
        </button>} />

      {error && <div role="alert" className="mt-4 flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-[11px] text-destructive">
        <AlertCircle className="mt-0.5 shrink-0" size={12}/><span className="min-w-0 break-words">{error}</span>
        <button type="button" onClick={() => void refresh()} className="ml-auto shrink-0 underline underline-offset-2">Retry</button>
      </div>}

      {loading && !agents && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-muted-foreground">
        <LoaderCircle className="animate-spin" size={15}/> Loading agents…
      </div>}

      {agents && <div className="mt-5 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
        {agents.map(agent => <AgentCard
          key={agent.agentId}
          agent={agent}
          busy={busy[agent.agentId] ?? null}
          error={failures[agent.agentId]}
          onInstall={() => void run(agent.agentId, "Installing", bridgeApi.installManagedAgent)}
          onUninstall={() => void run(agent.agentId, "Uninstalling", bridgeApi.uninstallManagedAgent)}
        />)}
      </div>}
    </div>
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

  return <article className="flex items-center gap-3.5 px-4 py-4" data-testid={`agent-card-${agent.agentId}`}>
    <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-[13px] border border-border bg-accent font-display text-[13px] font-semibold text-foreground">
      {agent.label.slice(0, 2).toUpperCase()}
    </div>

    <div className="min-w-0 flex-1">
      <h3 className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-foreground">{agent.label}</h3>
      <p className="mt-0.5 truncate text-[12px] text-muted-foreground">
        {BLURB[agent.agentId] ?? "Coding agent"}{agent.version ? ` · ${agent.version}` : ""}
      </p>
      {error && <p role="alert" className="mt-1 text-[11px] text-destructive">{error}</p>}
    </div>

    {busy
      ? <span role="status" aria-live="polite" data-testid={`agent-busy-${agent.agentId}`}
          className="shrink-0 text-[11px] text-muted-foreground motion-safe:animate-pulse">{busy}…</span>
      : installed
        ? <button type="button" onClick={onUninstall}
            className="shrink-0 min-h-8 rounded-lg border border-input px-3 py-1 text-[12px] font-medium text-foreground transition-colors hover:bg-accent">
            Uninstall
          </button>
        : <button type="button" onClick={onInstall}
            className="shrink-0 min-h-8 rounded-lg bg-primary px-3 py-1 text-[12px] font-medium text-primary-foreground transition-opacity hover:opacity-90">
            Install
          </button>}
  </article>;
}
