import { useEffect, useState } from "react";
import { LoaderCircle, Search, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { SearchSessionEntriesResult, SessionRecallHit } from "../types";

export type SessionRecallSearchProps = {
  sessionId: string;
  search?: (sessionId: string, query: string) => Promise<SearchSessionEntriesResult>;
  onClose: () => void;
  onJump: (entryId: string) => void;
};

export function SessionRecallSearch({
  sessionId,
  search = (id, query) => bridgeApi.searchSessionEntries(id, query),
  onClose,
  onJump,
}: SessionRecallSearchProps) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SessionRecallHit[]>([]);
  const [status, setStatus] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      setHits([]);
      setStatus("idle");
      setError(null);
      return;
    }
    let cancelled = false;
    setStatus("loading");
    const handle = window.setTimeout(() => {
      void search(sessionId, trimmed)
        .then(result => {
          if (cancelled) return;
          setHits(result.hits);
          setStatus("ready");
          setError(null);
        })
        .catch(cause => {
          if (cancelled) return;
          setHits([]);
          setStatus("error");
          setError(cause instanceof Error ? cause.message : String(cause));
        });
    }, 180);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [query, search, sessionId]);

  return (
    <div className="shrink-0 border-b border-border px-3 py-2 sm:px-6">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-2">
        <div className="flex items-center gap-2">
          <label className="u-glass-soft flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2.5">
            <Search size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={event => setQuery(event.target.value)}
              autoFocus
              placeholder="Search this chat…"
              aria-label="Search this chat"
              className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
            />
            {status === "loading" && <LoaderCircle size={13} className="shrink-0 animate-spin text-muted-foreground" aria-hidden="true" />}
          </label>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close search"
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <X size={14} strokeWidth={1.8} aria-hidden="true" />
          </button>
        </div>
        <p className="text-[11px] text-muted-foreground">This search stays in this chat. It does not look at other sessions or account memory.</p>
        {status === "error" && error && <p className="text-[12px] text-destructive">{error}</p>}
        {status === "ready" && hits.length === 0 && (
          <p className="text-[12px] text-muted-foreground">No matches in this chat.</p>
        )}
        {hits.length > 0 && (
          <ul className="max-h-48 overflow-y-auto">
            {hits.map(hit => (
              <li key={hit.entryId}>
                <button
                  type="button"
                  onClick={() => onJump(hit.entryId)}
                  className="flex w-full flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent"
                >
                  <span className="font-mono text-[11px] text-muted-foreground">{hit.kind} · #{hit.sequence}</span>
                  <span className="line-clamp-2 text-[13px] text-foreground">{hit.snippet}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
