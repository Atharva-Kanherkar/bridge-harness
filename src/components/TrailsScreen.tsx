import { useCallback, useEffect, useRef, useState } from "react";
import type { TrailDetail, TrailEvent, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { friendlyError } from "@/errors";
import { TrailsList, type TrailWorkspace } from "./trails/TrailsList";
import { TrailPage } from "./trails/TrailPage";
import { TrailStartDialog, type TrailHarnessOption, type TrailStartRequest } from "./trails/TrailStartDialog";
import { TrailAddDialog } from "./trails/TrailAddDialog";
import { useLatest } from "../useLatest";

export type { TrailStartRequest } from "./trails/TrailStartDialog";

/** The Trails destination: the list, or one trail's page when `trailId` is set. */
export function TrailsScreen({ trailId, knownTrails, onSelectTrail, workspaces, defaultWorkspaceId, harnesses, defaultHarness, onOpenSession, onOpenNote, onOpenUrl, onStart }: {
  trailId: string | null;
  /** Summaries the host already holds: a trail opened from elsewhere paints its header from them at once. */
  knownTrails?: readonly TrailSummary[];
  onSelectTrail: (trailId: string | null) => void;
  workspaces: TrailWorkspace[];
  defaultWorkspaceId?: string | null;
  harnesses: TrailHarnessOption[];
  defaultHarness?: string | null;
  onOpenSession: (sessionId: string) => void;
  onOpenNote: (pageId: string) => void;
  onOpenUrl: (url: string) => void;
  /** Create the chat, carry the brief, send the first message. */
  onStart: (request: TrailStartRequest) => Promise<void>;
}) {
  const [trails, setTrails] = useState<TrailSummary[] | null>(null);
  const [listError, setListError] = useState<string>();
  const [showArchived, setShowArchived] = useState(false);
  const [detail, setDetail] = useState<TrailDetail | null>(null);
  const [detailError, setDetailError] = useState<string>();
  const [starting, setStarting] = useState(false);
  const [adding, setAdding] = useState(false);
  // A GitHub re-read in flight for the trail on screen: Refresh spins.
  const [refreshing, setRefreshing] = useState(false);

  const loadList = useCallback(() => {
    bridgeApi.listTrails(true)
      .then(result => { setTrails(result.trails); setListError(undefined); })
      .catch(cause => setListError(friendlyError(cause, "Couldn't load your trails.")));
  }, []);

  // A read answers for the trail it asked about. Only the newest request for
  // the trail still on screen may land, success or error: a slow refresh of a
  // trail the person left must not replace the one rename and archive act on.
  const selectedRef = useLatest(trailId);
  const requestRef = useRef(0);
  // The newest GitHub re-read: only its settling stops the spinner.
  const refreshRef = useRef(0);
  const loadDetail = useCallback(function loadDetail(id: string, refresh = false): Promise<void> {
    const request = ++requestRef.current;
    const current = () => request === requestRef.current && selectedRef.current === id;
    if (refresh) { refreshRef.current = request; setRefreshing(true); }
    return bridgeApi.readTrail(id, refresh)
      .then(result => {
        if (current()) { setDetail(result.trail); setDetailError(undefined); }
        // GitHub's answer lost to a newer local read, which may have run
        // before the refresh stored what it found: read once more so that
        // reaches the screen. A newer refresh carries it anyway.
        else if (refresh && refreshRef.current === request && selectedRef.current === id) void loadDetail(id);
      })
      .catch(cause => { if (current()) setDetailError(friendlyError(cause, "Couldn't load this trail.")); })
      .finally(() => { if (refresh && refreshRef.current === request) setRefreshing(false); });
  }, [selectedRef]);

  useEffect(() => { loadList(); }, [loadList]);
  useEffect(() => {
    setDetail(current => current?.summary.id === trailId ? current : null);
    setDetailError(undefined);
    refreshRef.current = 0;
    setRefreshing(false);
    if (!trailId) return;
    // Paint from what Bridge has stored, then re-read the trail's pull
    // requests and issues from GitHub once, behind the page: that read runs
    // `gh` per pull request and can take seconds.
    let live = true;
    void loadDetail(trailId).then(() => { if (live) void loadDetail(trailId, true); });
    return () => { live = false; };
  }, [trailId, loadDetail]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let live = true;
    void bridgeApi.onTrailsChanged(payload => {
      loadList();
      if (trailId && (!payload.trailId || payload.trailId === trailId)) loadDetail(trailId);
    }).then(stop => { if (live) unlisten = stop; else stop(); });
    return () => { live = false; unlisten?.(); };
  }, [trailId, loadList, loadDetail]);

  const titleOf = (id?: string | null) => workspaces.find(workspace => workspace.id === id)?.title;

  function openEvent(event: TrailEvent) {
    if (event.member.kind === "note") onOpenNote(event.member.ref);
    else if ((event.member.kind === "pullRequest" || event.member.kind === "issue") && event.url) onOpenUrl(event.url);
    else if (event.sessionId) onOpenSession(event.sessionId);
  }

  async function remove(event: TrailEvent) {
    if (!detail || detail.summary.id !== trailId) return;
    try {
      await bridgeApi.removeTrailMember(detail.summary.id, event.member);
      loadDetail(detail.summary.id);
    } catch (cause) {
      setDetailError(friendlyError(cause, "Couldn't remove that from the trail."));
    }
  }

  if (trailId) {
    const shown = detail?.summary.id === trailId ? detail : null;
    // What the page can say before its detail arrives.
    const summary = shown?.summary ?? trails?.find(trail => trail.id === trailId) ?? knownTrails?.find(trail => trail.id === trailId) ?? null;
    return <>
      <TrailPage
        trail={shown}
        summary={summary}
        workspaceTitle={titleOf(summary?.workspaceId)}
        loading={!shown && !detailError}
        refreshing={refreshing}
        error={detailError}
        onRetry={() => void loadDetail(trailId)}
        onBack={() => onSelectTrail(null)}
        onContinue={() => setStarting(true)}
        onOpenEvent={openEvent}
        onRemove={event => void remove(event)}
        onRename={async title => {
          try { await bridgeApi.updateTrail(trailId, { title }); loadDetail(trailId); }
          catch (cause) { setDetailError(friendlyError(cause, "Couldn't rename the trail.")); }
        }}
        onArchive={archived => {
          void bridgeApi.updateTrail(trailId, { archived })
            .then(() => { if (archived) onSelectTrail(null); else loadDetail(trailId); })
            .catch(cause => setDetailError(friendlyError(cause, archived ? "Couldn't archive the trail." : "Couldn't restore the trail.")));
        }}
        onRefresh={() => loadDetail(trailId, true)}
        onAdd={() => setAdding(true)}
      />
      {adding && summary && (
        <TrailAddDialog
          trail={summary}
          onClose={() => setAdding(false)}
          onAdd={async member => {
            await bridgeApi.addTrailMember(trailId, member);
            loadDetail(trailId);
          }}
          onAddIssue={async reference => {
            await bridgeApi.addTrailMember(trailId, { kind: "issue", ref: reference });
            loadDetail(trailId);
          }}
          onCreateNote={async title => {
            const note = await bridgeApi.createNotePage({ title, workspaceId: summary.workspaceId ?? undefined });
            await bridgeApi.addTrailMember(trailId, { kind: "note", ref: note.id });
            setAdding(false);
            onOpenNote(note.id);
          }}
        />
      )}
      {starting && summary && (
        <TrailStartDialog
          trail={summary}
          harnesses={harnesses}
          defaultHarness={defaultHarness}
          onClose={() => setStarting(false)}
          onStart={async request => { await onStart(request); setStarting(false); }}
        />
      )}
    </>;
  }

  return (
    <TrailsList
      trails={trails}
      loading={!trails && !listError}
      error={listError}
      workspaces={workspaces}
      defaultWorkspaceId={defaultWorkspaceId}
      showArchived={showArchived}
      onToggleArchived={() => setShowArchived(value => !value)}
      onOpen={id => onSelectTrail(id)}
      onCreate={async (title, workspaceId) => {
        const created = await bridgeApi.createTrail(title, workspaceId);
        loadList();
        onSelectTrail(created.trail.id);
      }}
    />
  );
}
