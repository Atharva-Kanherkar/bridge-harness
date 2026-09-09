import { useEffect, useState } from "react";
import { Archive, ArrowLeft, ArchiveRestore } from "lucide-react";
import { bridgeApi } from "../../api";
import type { ArchivedChat } from "../../protocol/generated/protocol";
import type { AgentEvent } from "../../types";
import { GhostButton, SettingsGroup, SettingsPage } from "./kit";

export function ArchivedChatsPage() {
  const [query, setQuery] = useState("");
  const [offset, setOffset] = useState(0);
  const [chats, setChats] = useState<ArchivedChat[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [selected, setSelected] = useState<ArchivedChat>();
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [note, setNote] = useState<string>();
  const [revision, setRevision] = useState(0);
  const [sequence, setSequence] = useState(0);
  const [moreContent, setMoreContent] = useState(false);

  useEffect(() => {
    let active = true;
    setLoading(true); setError(undefined);
    const request = selected
      ? bridgeApi.replaySessionEvents(selected.id, sequence, 200).then(page => {
          if (!active) return;
          setEvents(page);
          setMoreContent(page.length === 200);
        })
      : bridgeApi.listArchivedChats(query, offset).then(page => {
          if (!active) return;
          setChats(page.chats); setHasMore(page.hasMore);
        });
    void request.catch(cause => { if (active) setError(String(cause)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [query, offset, selected, revision, sequence]);

  const unarchive = async (chat: ArchivedChat) => {
    setBusy(true); setError(undefined);
    try {
      await bridgeApi.unarchiveChat(chat.id);
      setNote(`"${chat.title}" returned to chat history. Its checkout was not restored.`);
      setSelected(undefined); setOffset(0); setRevision(value => value + 1);
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };

  return <SettingsPage title="Archived chats" description="Your conversations, kept for reference. Unarchive returns a chat to history without restoring its worktree or starting a model.">
    {error && <p role="alert" className="rounded-lg border border-destructive/30 p-3 text-sm text-destructive">{error} <button type="button" onClick={() => setRevision(value => value + 1)} className="underline">Retry</button></p>}
    {note && <p role="status" className="text-sm text-muted-foreground">{note}</p>}
    {selected ? <>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <GhostButton onClick={() => setSelected(undefined)}><ArrowLeft size={14} /> All archived chats</GhostButton>
        <GhostButton disabled={busy} onClick={() => void unarchive(selected)}><ArchiveRestore size={14} />{busy ? "Unarchiving..." : "Unarchive"}</GhostButton>
      </div>
      <SettingsGroup label={selected.title} note="Read-only transcript. Tools and system events are omitted; no model is running.">
        {loading ? <p role="status" className="p-4 text-sm text-muted-foreground">Loading conversation...</p> : <div className="divide-y divide-border">
          {events.filter(event => (event.role === "user" || event.role === "assistant") && event.text).map(event => <article key={`${event.sequence}:${event.id}`} className="p-4">
            <p className="mb-2 text-xs font-semibold uppercase text-muted-foreground">{event.role}</p>
            <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-foreground">{event.text}</p>
          </article>)}
          {events.length === 0 && <p className="p-4 text-sm text-muted-foreground">No recorded messages.</p>}
        </div>}
      </SettingsGroup>
      <div className="flex gap-2">
        {sequence > 0 && <GhostButton disabled={loading} onClick={() => setSequence(0)}>Beginning</GhostButton>}
        {moreContent && <GhostButton disabled={loading} onClick={() => setSequence(events[events.length - 1]?.sequence ?? sequence)}>Next messages</GhostButton>}
      </div>
    </> : <>
      <input type="search" aria-label="Search archived chats" placeholder="Search by chat or project name" value={query} onChange={event => { setQuery(event.target.value); setOffset(0); }} className="h-10 w-full rounded-lg border border-border bg-card px-3 text-sm outline-none focus:border-ring" />
      {loading ? <p role="status" className="text-sm text-muted-foreground">Loading archived chats...</p> : <SettingsGroup label="Conversations">
        {chats.length === 0 ? <div className="p-8 text-center text-muted-foreground"><Archive size={24} className="mx-auto mb-3" /><p className="text-sm">{query ? "No archived chats match your search." : "No archived chats yet."}</p></div> : <ul className="divide-y divide-border">
          {chats.map(chat => <li key={chat.id} className="flex flex-wrap items-center gap-3 p-4">
            <button type="button" className="min-w-0 flex-1 text-left" onClick={() => { setSelected(chat); setSequence(0); setEvents([]); }}>
              <span className="block truncate text-sm font-medium text-foreground">{chat.title}</span>
              <span className="mt-1 block text-xs text-muted-foreground">{chat.workspaceTitle ?? "Direct chat"} · {chat.harness} · {new Date(chat.archivedAt).toLocaleDateString()}</span>
            </button>
            <GhostButton disabled={busy} ariaLabel={`Unarchive ${chat.title}`} onClick={() => void unarchive(chat)}><ArchiveRestore size={14} />Unarchive</GhostButton>
          </li>)}
        </ul>}
      </SettingsGroup>}
      <div className="flex items-center gap-3">
        {offset > 0 && <GhostButton disabled={loading} onClick={() => setOffset(value => Math.max(0, value - 50))}>Previous</GhostButton>}
        {hasMore && <GhostButton disabled={loading} onClick={() => setOffset(value => value + 50)}>Next page</GhostButton>}
      </div>
    </>}
  </SettingsPage>;
}
