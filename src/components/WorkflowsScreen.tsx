import { useCallback, useEffect, useMemo, useState } from "react";
import { ExternalLink, FolderGit2, Play, Plus, Settings2, Sparkles, Workflow as WorkflowIcon } from "lucide-react";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import type { ForkSessionResult, Project, Workflow, WorkflowDefinition, WorkflowDraft, WorkflowOptions, WorkflowRun, WorkflowSettingsSnapshot, WorkflowSummary, WorkflowTemplate } from "../protocol/generated/protocol";
import { openExternalUrl } from "../externalLinks";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { AutomationsPanel } from "./AutomationsPanel";
import { WorkflowDetail } from "./workflows/WorkflowDetail";
import { WorkflowEditor, type EditorSubmit } from "./workflows/WorkflowEditor";
import { CLAUDE_CONNECTORS_URL, WorkflowRunView } from "./workflows/WorkflowRunView";
import { ACTIVE_RUN_STATUSES, RUN_STATUS_LABEL, costLine, relativeTime, runTone, triggerSummary } from "./workflows/format";
import { WorkflowTemplates } from "./workflows/WorkflowTemplates";
import { CHIP, Dot, ErrorNote, StatusChip } from "./workflows/parts";
import { WorkflowInbox } from "./workflows/WorkflowInbox";
import { needsYouCount } from "./workflows/notify";
import { RowButton } from "@/components/ui/row";
import { SegmentedGroup, SegmentedItem } from "@/components/ui/segmented";
import { Checkbox } from "@/components/ui/checkbox";
import { SkeletonRows } from "@/components/ui/skeleton";
import { useNow } from "../useNow";
import { useLatest } from "../useLatest";

type Tab = "workflows" | "templates" | "inbox" | "native";
type Editing = { workflow: Workflow | null; initial?: WorkflowDefinition | null; notes?: string[] } | null;

const requestId = () => (globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`);

/** Granted connectors Claude says cannot be reached, as "Slack needs sign-in". */
export function connectorTrouble(row: WorkflowSummary): string | null {
  const blocked = (row.connectors ?? []).filter(status => status.state === "needs_sign_in" || status.state === "missing");
  if (blocked.length === 0) return null;
  return blocked.map(status => `${status.server.replace(/^claude\.ai /, "")} ${status.state === "missing" ? "is no longer connected" : "needs sign-in"}`).join(", ");
}

/** Each queued run's place in line, oldest first. */
export function queuePositions(runs: WorkflowRun[]): Map<string, number> {
  const queued = runs.filter(run => run.status === "queued" && !run.sessionId).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  return new Map(queued.map((run, index) => [run.id, index + 1]));
}

function summaryStatus(row: WorkflowSummary): { label: string; tone: "ok" | "warn" | "idle" } {
  if (!row.approved) return { label: "needs approval", tone: "warn" };
  if (!row.enabled) return { label: row.disabledReason === "project_removed" ? "project gone" : "paused", tone: "warn" };
  return { label: "active", tone: "ok" };
}

export function WorkflowsScreen({ projects, onContinueInChat, initialTab = "workflows", initialDraft = null, onDraftOpened, focusRun = null, onRunFocused }: {
  projects: Project[];
  onContinueInChat: (result: ForkSessionResult) => void;
  initialTab?: Tab;
  /** A draft made elsewhere (Save as workflow), opened in the editor once. */
  initialDraft?: WorkflowDraft | null;
  onDraftOpened?: () => void;
  /** A run to open on arrival (from Mission Control or a notification). */
  focusRun?: { workflowId: string; runId: string } | null;
  onRunFocused?: () => void;
}) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [rows, setRows] = useState<WorkflowSummary[] | null>(null);
  const [options, setOptions] = useState<WorkflowOptions | null>(null);
  const [selected, setSelected] = useState<Workflow | null>(null);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [openRun, setOpenRun] = useState<WorkflowRun | null>(null);
  const [editing, setEditing] = useState<Editing>(null);
  const [settings, setSettings] = useState<WorkflowSettingsSnapshot | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [allRuns, setAllRuns] = useState<WorkflowRun[]>([]);
  const [describe, setDescribe] = useState("");
  const [drafting, setDrafting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const selectedId = useLatest(selected?.id ?? null);
  const now = useNow(60_000);

  const refreshList = useCallback(async () => {
    try {
      const [list, recent] = await Promise.all([bridgeApi.listWorkflows(), bridgeApi.listWorkflowRuns({ limit: 100 })]);
      setAllRuns(recent.runs);
      setRows(list.workflows);
    } catch (reason) {
      setError(friendlyError(reason, "Couldn't load the workflows."));
      setRows(current => current ?? []);
    }
  }, []);

  const refreshSelected = useCallback(async (workflowId: string) => {
    const [snapshot, history] = await Promise.all([
      bridgeApi.getWorkflow(workflowId),
      bridgeApi.listWorkflowRuns({ workflowId, limit: 20 }),
    ]);
    setSelected(snapshot.workflow);
    setRuns(history.runs);
    setOpenRun(current => current ? history.runs.find(run => run.id === current.id) ?? current : current);
  }, []);

  useEffect(() => {
    void refreshList();
    bridgeApi.workflowOptions().then(setOptions).catch(reason => setError(friendlyError(reason, "Couldn't load the workflow options.")));
    bridgeApi.readWorkflowSettings().then(setSettings).catch(() => {});
  }, [refreshList]);

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    const follow = () => {
      void refreshList();
      const id = selectedId.current;
      if (id) void refreshSelected(id).catch(() => {});
    };
    void bridgeApi.onWorkflowsChanged(follow).then(stop => disposed ? stop() : unlisten.push(stop));
    void bridgeApi.onWorkflowRunChanged(follow).then(stop => disposed ? stop() : unlisten.push(stop));
    return () => { disposed = true; unlisten.forEach(stop => stop()); };
  }, [refreshList, refreshSelected, selectedId]);

  useEffect(() => {
    if (!initialDraft) return;
    setEditorError(null);
    setSelected(null);
    setEditing({ workflow: null, initial: initialDraft.definition, notes: initialDraft.notes });
    onDraftOpened?.();
  }, [initialDraft, onDraftOpened]);

  const draftFromText = async () => {
    const text = describe.trim();
    if (!text) return;
    setDrafting(true);
    setError(null);
    try {
      const { draft } = await bridgeApi.draftWorkflowFromText(text);
      setEditorError(null);
      setEditing({ workflow: null, initial: draft.definition, notes: draft.notes });
      setDescribe("");
    } catch (reason) {
      setError(friendlyError(reason, "Couldn't draft a workflow from that description."));
    } finally {
      setDrafting(false);
    }
  };

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
      await refreshList();
      if (selectedId.current) await refreshSelected(selectedId.current);
    } catch (reason) {
      setError(friendlyError(reason, "Something went wrong with the workflow. Try again."));
    } finally {
      setBusy(false);
    }
  };

  const open = (id: string) => void act(() => refreshSelected(id));
  // Opening a run reads it (see the effect below): the inbox and the sidebar
  // count stop asking.
  const openRunOf = useCallback(async (run: WorkflowRun) => {
    await refreshSelected(run.workflowId);
    setOpenRun(run);
  }, [refreshSelected]);

  // A finished run that is open has been seen: opened from a list, or watched
  // until it finished.
  useEffect(() => {
    if (!openRun?.unread || ACTIVE_RUN_STATUSES.includes(openRun.status)) return;
    const id = openRun.id;
    void bridgeApi.markWorkflowRunsRead([id]).then(() => {
      setOpenRun(current => current?.id === id ? { ...current, unread: false } : current);
      setAllRuns(current => current.map(candidate => candidate.id === id ? { ...candidate, unread: false } : candidate));
    }).catch(() => {});
  }, [openRun?.id, openRun?.unread, openRun?.status]);

  useEffect(() => {
    if (!focusRun) return;
    bridgeApi.listWorkflowRuns({ workflowId: focusRun.workflowId, limit: 50 })
      .then(({ runs: found }) => {
        const run = found.find(candidate => candidate.id === focusRun.runId);
        if (run) void openRunOf(run);
      })
      .catch(reason => setError(friendlyError(reason, "Couldn't open that workflow run.")))
      .finally(() => onRunFocused?.());
  }, [focusRun, onRunFocused, openRunOf]);
  const runWorkflow = (workflowId: string, dryRun: boolean) => act(async () => {
    const result = await bridgeApi.runWorkflow(workflowId, requestId(), dryRun);
    await refreshSelected(workflowId);
    setOpenRun(result.run);
  });

  const refreshConnectors = async () => {
    setRefreshing(true);
    try {
      setOptions(await bridgeApi.refreshWorkflowConnectors());
    } catch (reason) {
      setEditorError(friendlyError(reason, "Couldn't refresh the connectors."));
    } finally {
      setRefreshing(false);
    }
  };

  // Try a template: save it unapproved (nothing runs on its own) and dry-run
  // it, so the person sees what it would do before approving anything.
  const tryTemplate = (template: WorkflowTemplate) => act(async () => {
    const saved = await bridgeApi.saveWorkflow(null, template.definition);
    let event = null;
    if (template.definition.trigger.kind === "event") {
      const preview = await bridgeApi.previewWorkflowTrigger(template.definition.trigger).catch(() => null);
      event = preview?.samples[0] ?? null;
    }
    const result = await bridgeApi.runWorkflow(saved.workflow.id, requestId(), true, event);
    await refreshList();
    await refreshSelected(saved.workflow.id);
    setTab("workflows");
    setOpenRun(result.run);
  });

  const customizeTemplate = (template: WorkflowTemplate) => {
    const notes = [
      ...template.missing.map(app => `Connect ${app[0].toUpperCase()}${app.slice(1)} in Claude first; until then this workflow cannot reach it.`),
      ...(template.needsProject ? ["Choose the project it works on."] : []),
    ];
    setEditorError(null);
    setSelected(null);
    setEditing({ workflow: null, initial: template.definition, notes });
  };

  const submitEditor = async ({ definition, approve, dryRun, event }: EditorSubmit) => {
    setBusy(true);
    setEditorError(null);
    try {
      const saved = await bridgeApi.saveWorkflow(editing?.workflow?.id ?? null, definition);
      let workflow = saved.workflow;
      if (approve && saved.needsApproval) workflow = (await bridgeApi.approveWorkflow(workflow.id, workflow.revisionId)).workflow;
      setEditing(null);
      await refreshList();
      await refreshSelected(workflow.id);
      if (dryRun) {
        const result = await bridgeApi.runWorkflow(workflow.id, requestId(), true, event ?? null);
        setOpenRun(result.run);
      }
    } catch (reason) {
      setEditorError(friendlyError(reason, "Couldn't save the workflow."));
    } finally {
      setBusy(false);
    }
  };

  const projectName = useMemo(() => new Map(projects.map(project => [project.id, project.name])), [projects]);
  const heading = <ScreenHeading title="Workflows" action={<>
    <Button size="xs" disabled={busy} onClick={() => { setEditorError(null); setEditing({ workflow: null }); }}><Plus size={13}/>New workflow</Button>
    <Button size="xs" variant="secondary" onClick={() => setSettingsOpen(true)}><Settings2 size={13}/>Settings</Button>
  </>}/>;

  let body;
  if (selected && openRun) {
    body = <WorkflowRunView workflow={selected} run={openRun} onBack={() => setOpenRun(null)} onContinue={onContinueInChat} queuePosition={queuePositions(allRuns).get(openRun.id) ?? null}/>;
  } else if (selected) {
    const summary = rows?.find(row => row.id === selected.id);
    body = <WorkflowDetail workflow={selected} runs={runs} projects={projects} nextRunAt={summary?.nextRunAt ?? null} busy={busy}
      onBack={() => setSelected(null)}
      onRun={dryRun => void runWorkflow(selected.id, dryRun)}
      onToggle={() => void act(() => bridgeApi.setWorkflowEnabled(selected.id, !selected.enabled))}
      onApprove={() => void act(() => bridgeApi.approveWorkflow(selected.id, selected.revisionId))}
      onEdit={() => { setEditorError(null); setEditing({ workflow: selected }); }}
      onDelete={() => setConfirmDelete(true)}
      onOpenRun={run => void act(() => openRunOf(run))}/>;
  } else {
    body = <>
      {heading}
      <form className="mb-5 flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2" onSubmit={event => { event.preventDefault(); void draftFromText(); }}>
        <Sparkles size={14} className="shrink-0 text-muted-foreground"/>
        <Input unstyled className="min-w-0 flex-1" aria-label="Describe a job" value={describe} onChange={event => setDescribe(event.target.value)} disabled={drafting}
          placeholder={'Describe a job: "every weekday at 9, summarise my Slack mentions and Linear issues"'}/>
        <Button size="xs" type="submit" loading={drafting} disabled={!describe.trim()}>Draft it</Button>
      </form>
      <SegmentedGroup tabs label="Workflows sections" className="w-fit">
        <SegmentedItem tab active={tab === "workflows"} onClick={() => setTab("workflows")}>Workflows</SegmentedItem>
        <SegmentedItem tab active={tab === "templates"} onClick={() => setTab("templates")}>Templates</SegmentedItem>
        <SegmentedItem tab active={tab === "inbox"} onClick={() => setTab("inbox")}>
          <span className="inline-flex items-center gap-1.5">Inbox{needsYouCount(allRuns) > 0 && <span aria-label={`${needsYouCount(allRuns)} need you`} className="rounded-full bg-foreground px-1.5 text-micro font-semibold leading-4 text-background">{needsYouCount(allRuns)}</span>}</span>
        </SegmentedItem>
        <SegmentedItem tab active={tab === "native"} onClick={() => setTab("native")}>Native schedules</SegmentedItem>
      </SegmentedGroup>
      {tab === "native" ? <div className="mt-4"><AutomationsPanel embedded/></div>
        : tab === "templates" ? <WorkflowTemplates busy={busy} onTry={template => void tryTemplate(template)} onCustomize={customizeTemplate}/>
        : tab === "inbox" ? <WorkflowInbox runs={allRuns} names={new Map((rows ?? []).map(row => [row.id, row.name]))} onOpenRun={run => void act(() => openRunOf(run))} onChanged={() => void refreshList()}/>
        : rows === null
        ? <SkeletonRows label="Loading workflows" rows={4} lead="icon" trailing className="mt-4" />
        : rows.length === 0
          ? <div className="mt-6 flex flex-col items-center rounded-2xl border border-dashed border-border px-6 py-12 text-center">
            <div className="flex h-11 w-11 items-center justify-center rounded-xl border border-border bg-accent text-muted-foreground"><WorkflowIcon size={18}/></div>
            <h2 className="mt-4 text-message font-semibold text-foreground">Automate a job you do every day</h2>
            <Button size="sm" className="mt-5" onClick={() => setEditing({ workflow: null })}><Plus size={13}/>New workflow</Button>
          </div>
          : <div className="mt-4 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{rows.map(row => {
            const status = summaryStatus(row);
            const next = relativeTime(row.nextRunAt, now);
            const last = row.lastRun;
            const trouble = connectorTrouble(row);
            const cost = costLine(row.averageRunCostMicrousd, row.monthlyCostEstimateMicrousd);
            const needsYou = !row.approved || last?.status === "waiting_approval" || last?.status === "failed" || Boolean(trouble);
            return <article key={row.id} className="flex items-start gap-3.5 px-4 py-3.5 hover:bg-accent">
              <RowButton size="bare" tone="plain" className="flex-1 items-start" onClick={() => open(row.id)}><span className="flex min-w-0 flex-1 items-start gap-3.5 text-foreground">
                <div className="relative mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-border bg-accent text-muted-foreground">
                  <WorkflowIcon size={15}/>{needsYou && <span aria-label="Needs you" className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-foreground ring-2 ring-card"/>}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2"><h3 className="truncate text-ui font-semibold text-foreground">{row.name}</h3><StatusChip label={status.label} tone={status.tone}/></div>
                  <p className="mt-1 text-meta text-muted-foreground">{triggerSummary(row.trigger)}{row.files === "worktree" ? " · worktree" : ""}</p>
                  <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-muted-foreground">
                    <span className={CHIP}>{row.projectId ? <span className="inline-flex items-center gap-1"><FolderGit2 size={10}/>{projectName.get(row.projectId) ?? "missing project"}</span> : "No project"}</span>
                    <span className={CHIP}>{row.harness} · {row.model}</span>
                    {next && <span>next {next}</span>}
                    {last && <span className="inline-flex items-center gap-1.5"><Dot tone={runTone(last.status)}/>{RUN_STATUS_LABEL[last.status]} {relativeTime(last.finishedAt ?? last.createdAt, now)}</span>}
                    {cost && <span>{cost}</span>}
                  </div>
                  {trouble && <p className="mt-1.5 text-caption text-foreground">{trouble} in Claude.</p>}
                </div>
              </span></RowButton>
              {trouble
                ? <Button size="xs" variant="secondary" onClick={() => void openExternalUrl(CLAUDE_CONNECTORS_URL)}><ExternalLink size={10}/>Reconnect</Button>
                : <Button size="xs" variant="secondary" disabled={busy || !row.approved || !row.enabled} onClick={() => void runWorkflow(row.id, false)}><Play size={10}/>Run now</Button>}
            </article>;
          })}</div>}
    </>;
  }

  return <div className="h-full min-h-0 overflow-y-auto"><div className={SCREEN_CONTENT}>
    {body}
    <ErrorNote message={error}/>
    <WorkflowEditor key={editing ? (editing.workflow?.revisionId ?? (editing.initial ? `draft-${editing.initial.name}` : "new")) : "closed"} open={!!editing} workflow={editing?.workflow ?? null} initial={editing?.initial ?? null} notes={editing?.notes ?? []}
      options={options} projects={projects} busy={busy} error={editorError} onCancel={() => setEditing(null)} onSubmit={submit => void submitEditor(submit)}
      onRefreshConnectors={() => void refreshConnectors()} refreshing={refreshing}/>
    <Dialog open={confirmDelete} onOpenChange={next => { if (!next && !busy) setConfirmDelete(false); }}>{confirmDelete && selected && <DialogContent showCloseButton={!busy}>
      <DialogHeader><DialogTitle>Delete this workflow?</DialogTitle><DialogDescription>Its runs and their record go too. Chats you continued from a run stay.</DialogDescription></DialogHeader>
      <DialogPanel><p className="rounded-xl border border-border bg-muted p-3 text-caption text-foreground">{selected.definition.name}</p></DialogPanel>
      <DialogFooter><Button variant="ghost" disabled={busy} onClick={() => setConfirmDelete(false)}>Cancel</Button><Button variant="destructive" disabled={busy} onClick={() => void act(async () => {
        await bridgeApi.deleteWorkflow(selected.id);
        setConfirmDelete(false);
        setSelected(null);
      })}>Delete</Button></DialogFooter>
    </DialogContent>}</Dialog>
    <WorkflowSettingsDialog open={settingsOpen} snapshot={settings} onClose={() => setSettingsOpen(false)} onSaved={next => { setSettings(next); void refreshList(); }}/>
  </div></div>;
}

function WorkflowSettingsDialog({ open, snapshot, onClose, onSaved }: {
  open: boolean;
  snapshot: WorkflowSettingsSnapshot | null;
  onClose: () => void;
  onSaved: (next: WorkflowSettingsSnapshot) => void;
}) {
  const [paused, setPaused] = useState(snapshot?.settings.paused ?? false);
  const [reserve, setReserve] = useState(snapshot?.settings.quotaReservePercent ?? 25);
  const [concurrent, setConcurrent] = useState(snapshot?.settings.maxConcurrentRuns ?? 2);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    setPaused(snapshot?.settings.paused ?? false);
    setReserve(snapshot?.settings.quotaReservePercent ?? 25);
    setConcurrent(snapshot?.settings.maxConcurrentRuns ?? 2);
    setError(null);
  }, [open, snapshot]);
  const save = async () => {
    try {
      onSaved(await bridgeApi.writeWorkflowSettings({ paused, quotaReservePercent: reserve, maxConcurrentRuns: concurrent }));
      onClose();
    } catch (reason) {
      setError(friendlyError(reason, "Couldn't save the workflow settings."));
    }
  };
  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>{open && <DialogContent>
    <DialogHeader><DialogTitle>Workflow settings</DialogTitle><DialogDescription>These apply to every workflow.</DialogDescription></DialogHeader>
    <DialogPanel><div className="space-y-3">
      <label className="flex items-center gap-2 text-ui text-foreground"><Checkbox aria-label="Pause all workflows" checked={paused} onCheckedChange={checked => setPaused(checked === true)}/>Pause all workflows</label>
      <label className="block text-caption font-medium text-muted-foreground">Keep this much of each plan for my own chats (percent)
        <Input aria-label="Quota reserve" type="number" min={0} max={90} className="mt-1 w-28" value={reserve} onChange={event => setReserve(Number(event.target.value))}/>
      </label>
      <p className="text-caption text-muted-foreground">Workflows wait while a provider is above {100 - reserve}% of any usage window.</p>
      <label className="block text-caption font-medium text-muted-foreground">Run at most this many workflows at once
        <Input aria-label="Workflows at once" type="number" min={1} max={5} className="mt-1 w-28" value={concurrent} onChange={event => setConcurrent(Number(event.target.value))}/>
      </label>
      <p className="text-caption text-muted-foreground">The rest wait in line and start as soon as one finishes.</p>
      <ErrorNote message={error}/>
    </div></DialogPanel>
    <DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button onClick={() => void save()}>Save</Button></DialogFooter>
  </DialogContent>}</Dialog>;
}
