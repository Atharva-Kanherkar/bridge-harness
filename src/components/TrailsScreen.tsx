import { useCallback, useEffect, useState } from "react";
import type { TrailDetail, TrailEvent, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { TrailsList, type TrailWorkspace } from "./trails/TrailsList";
import { TrailPage } from "./trails/TrailPage";
import { TrailStartDialog, type TrailHarnessOption, type TrailStartRequest } from "./trails/TrailStartDialog";

export type { TrailStartRequest } from "./trails/TrailStartDialog";

function message(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** The Trails destination: the list, or one trail's page when `trailId` is set. */
export function TrailsScreen({ trailId, onSelectTrail, workspaces, defaultWorkspaceId, harnesses, defaultHarness, onOpenSession, onOpenNote, onOpenUrl, onStart }: {
  trailId: string | null;
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

  const loadList = useCallback(() => {
    bridgeApi.listTrails(true)
      .then(result => { setTrails(result.trails); setListError(undefined); })
      .catch(cause => setListError(message(cause)));
  }, []);

  const loadDetail = useCallback((id: string, refresh = false) => {
    bridgeApi.readTrail(id, refresh)
      .then(result => { setDetail(result.trail); setDetailError(undefined); })
      .catch(cause => setDetailError(message(cause)));
  }, []);

  useEffect(() => { loadList(); }, [loadList]);
  useEffect(() => {
    setDetail(current => current?.summary.id === trailId ? current : null);
    setDetailError(undefined);
    // Opening a page re-reads its pull requests from GitHub once.
    if (trailId) loadDetail(trailId, true);
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
    else if (event.member.kind === "pullRequest" && event.url) onOpenUrl(event.url);
    else if (event.sessionId) onOpenSession(event.sessionId);
  }

  async function remove(event: TrailEvent) {
    if (!detail) return;
    try {
      await bridgeApi.removeTrailMember(detail.summary.id, event.member);
      loadDetail(detail.summary.id);
    } catch (cause) {
      setDetailError(message(cause));
    }
  }

  if (trailId) {
    return <>
      <TrailPage
        trail={detail}
        workspaceTitle={titleOf(detail?.summary.workspaceId)}
        loading={!detail && !detailError}
        error={detailError}
        onBack={() => onSelectTrail(null)}
        onContinue={() => setStarting(true)}
        onOpenEvent={openEvent}
        onRemove={event => void remove(event)}
        onRename={async title => {
          try { await bridgeApi.updateTrail(trailId, { title }); loadDetail(trailId); }
          catch (cause) { setDetailError(message(cause)); }
        }}
        onArchive={archived => {
          void bridgeApi.updateTrail(trailId, { archived })
            .then(() => { if (archived) onSelectTrail(null); else loadDetail(trailId); })
            .catch(cause => setDetailError(message(cause)));
        }}
        onRefresh={() => loadDetail(trailId, true)}
      />
      {starting && detail && (
        <TrailStartDialog
          trail={detail.summary}
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
