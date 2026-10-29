import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FolderGit2, LoaderCircle, Play, Plus, Settings2, Workflow as WorkflowIcon } from "lucide-react";
import { bridgeApi } from "../api";
import type { ForkSessionResult, Project, Workflow, WorkflowDefinition, WorkflowOptions, WorkflowRun, WorkflowSettingsSnapshot, WorkflowSummary } from "../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { AutomationsPanel } from "./AutomationsPanel";
import { WorkflowDetail } from "./workflows/WorkflowDetail";
import { WorkflowEditor, type EditorSubmit } from "./workflows/WorkflowEditor";
import { WorkflowRunView } from "./workflows/WorkflowRunView";
import { RUN_STATUS_LABEL, relativeTime, runTone, triggerSummary } from "./workflows/format";
import { CHIP, Dot, ErrorNote, StatusChip, errorText } from "./workflows/parts";

type Tab = "workflows" | "native";
type Editing = { workflow: Workflow | null; initial?: WorkflowDefinition | null } | null;

const requestId = () => (globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`);

function summaryStatus(row: WorkflowSummary): { label: string; tone: "ok" | "warn" | "idle" } {
  if (!row.approved) return { label: "needs approval", tone: "warn" };
  if (!row.enabled) return { label: row.disabledReason === "project_removed" ? "project gone" : "paused", tone: "warn" };
  return { label: "active", tone: "ok" };
}

export function WorkflowsScreen({ projects, onContinueInChat, initialTab = "workflows" }: {
  projects: Project[];
  onContinueInChat: (result: ForkSessionResult) => void;
  initialTab?: Tab;
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
  const selectedId = useRef<string | null>(null);
  selectedId.current = selected?.id ?? null;
  const now = Date.now();

  const refreshList = useCallback(async () => {
    try {
      setRows((await bridgeApi.listWorkflows()).workflows);
    } catch (reason) {
      setError(errorText(reason));
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
    bridgeApi.workflowOptions().then(setOptions).catch(reason => setError(errorText(reason)));
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
  }, [refreshList, refreshSelected]);

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
      await refreshList();
      if (selectedId.current) await refreshSelected(selectedId.current);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setBusy(false);
    }
  };

  const open = (id: string) => void act(() => refreshSelected(id));
  const runWorkflow = (workflowId: string, dryRun: boolean) => act(async () => {
    const result = await bridgeApi.runWorkflow(workflowId, requestId(), dryRun);
    await refreshSelected(workflowId);
    setOpenRun(result.run);
  });

  const submitEditor = async ({ definition, approve, dryRun }: EditorSubmit) => {
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
        const result = await bridgeApi.runWorkflow(workflow.id, requestId(), true);
        setOpenRun(result.run);
      }
    } catch (reason) {
      setEditorError(errorText(reason));
    } finally {
      setBusy(false);
    }
  };

  const projectName = useMemo(() => new Map(projects.map(project => [project.id, project.name])), [projects]);
  const heading = <ScreenHeading title="Workflows" description="Repeatable jobs that run on your own agents and connectors. Writes always ask first." action={<>
    <Button size="xs" disabled={busy} onClick={() => { setEditorError(null); setEditing({ workflow: null }); }}><Plus size={13}/>New workflow</Button>
    <Button size="xs" variant="secondary" onClick={() => setSettingsOpen(true)}><Settings2 size={13}/>Settings</Button>
  </>}/>;

  let body;
  if (selected && openRun) {
    body = <WorkflowRunView workflow={selected} run={openRun} onBack={() => setOpenRun(null)} onContinue={onContinueInChat}/>;
  } else if (selected) {
    const summary = rows?.find(row => row.id === selected.id);
    body = <WorkflowDetail workflow={selected} runs={runs} projects={projects} nextRunAt={summary?.nextRunAt ?? null} busy={busy}
      onBack={() => setSelected(null)}
      onRun={dryRun => void runWorkflow(selected.id, dryRun)}
      onToggle={() => void act(() => bridgeApi.setWorkflowEnabled(selected.id, !selected.enabled))}
      onApprove={() => void act(() => bridgeApi.approveWorkflow(selected.id, selected.revisionId))}
      onEdit={() => { setEditorError(null); setEditing({ workflow: selected }); }}
      onDelete={() => setConfirmDelete(true)}
      onOpenRun={setOpenRun}/>;
  } else {
    body = <>
      {heading}
      <div className="u-segmented w-fit" role="tablist" aria-label="Workflows sections">
        <button type="button" role="tab" data-active={tab === "workflows"} aria-selected={tab === "workflows"} className="u-segmented-item" onClick={() => setTab("workflows")}>Workflows</button>
        <button type="button" role="tab" data-active={tab === "native"} aria-selected={tab === "native"} className="u-segmented-item" onClick={() => setTab("native")}>Native schedules</button>
      </div>
      {tab === "native" ? <div className="mt-4"><AutomationsPanel embedded/></div> : rows === null
        ? <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={15}/>Loading workflows…</div>
        : rows.length === 0
          ? <div className="mt-6 flex flex-col items-center rounded-2xl border border-dashed border-border px-6 py-12 text-center">
            <div className="flex h-11 w-11 items-center justify-center rounded-[14px] border border-border bg-accent text-muted-foreground"><WorkflowIcon size={18}/></div>
            <h2 className="mt-4 text-[15px] font-semibold text-foreground">Automate a job you do every day</h2>
            <p className="mt-1 max-w-md text-ui leading-relaxed text-muted-foreground">It runs on the plan you already pay for, through the connectors your agent already has. No project needed.</p>
            <Button size="sm" className="mt-5" onClick={() => setEditing({ workflow: null })}><Plus size={13}/>New workflow</Button>
          </div>
          : <div className="mt-4 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{rows.map(row => {
            const status = summaryStatus(row);
            const next = relativeTime(row.nextRunAt, now);
            const last = row.lastRun;
            const needsYou = !row.approved || last?.status === "waiting_approval" || last?.status === "failed";
            return <article key={row.id} className="flex items-start gap-3.5 px-4 py-3.5 hover:bg-accent">
              <button type="button" onClick={() => open(row.id)} className="flex min-w-0 flex-1 items-start gap-3.5 text-left">
                <div className="relative mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-border bg-accent text-muted-foreground">
                  <WorkflowIcon size={15}/>{needsYou && <span aria-label="Needs you" className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-foreground ring-2 ring-card"/>}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2"><h3 className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-foreground">{row.name}</h3><StatusChip label={status.label} tone={status.tone}/></div>
                  <p className="mt-1 text-[12px] text-muted-foreground">{triggerSummary(row.trigger)}{row.files === "worktree" ? " · worktree" : ""}</p>
                  <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-muted-foreground">
                    <span className={CHIP}>{row.projectId ? <span className="inline-flex items-center gap-1"><FolderGit2 size={10}/>{projectName.get(row.projectId) ?? "missing project"}</span> : "No project"}</span>
                    <span className={CHIP}>{row.harness} · {row.model}</span>
                    {next && <span>next {next}</span>}
                    {last && <span className="inline-flex items-center gap-1.5"><Dot tone={runTone(last.status)}/>{RUN_STATUS_LABEL[last.status]} {relativeTime(last.finishedAt ?? last.createdAt, now)}</span>}
                  </div>
                </div>
              </button>
              <Button size="xs" variant="secondary" disabled={busy || !row.approved || !row.enabled} onClick={() => void runWorkflow(row.id, false)}><Play size={10}/>Run now</Button>
            </article>;
          })}</div>}
    </>;
  }

  return <div className="h-full min-h-0 overflow-y-auto"><div className={SCREEN_CONTENT}>
    {body}
    <ErrorNote message={error}/>
    <WorkflowEditor key={editing ? (editing.workflow?.revisionId ?? "new") : "closed"} open={!!editing} workflow={editing?.workflow ?? null} initial={editing?.initial ?? null}
      options={options} projects={projects} busy={busy} error={editorError} onCancel={() => setEditing(null)} onSubmit={submit => void submitEditor(submit)}/>
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
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    setPaused(snapshot?.settings.paused ?? false);
    setReserve(snapshot?.settings.quotaReservePercent ?? 25);
    setError(null);
  }, [open, snapshot]);
  const save = async () => {
    try {
      onSaved(await bridgeApi.writeWorkflowSettings({ paused, quotaReservePercent: reserve }));
      onClose();
    } catch (reason) {
      setError(errorText(reason));
    }
  };
  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>{open && <DialogContent>
    <DialogHeader><DialogTitle>Workflow settings</DialogTitle><DialogDescription>These apply to every workflow.</DialogDescription></DialogHeader>
    <DialogPanel className="space-y-3">
      <label className="flex items-center gap-2 text-[13px] text-foreground"><input type="checkbox" aria-label="Pause all workflows" checked={paused} onChange={event => setPaused(event.target.checked)} className="size-3.5 accent-foreground"/>Pause all workflows</label>
      <label className="block text-caption font-medium text-muted-foreground">Keep this much of each plan for my own chats (percent)
        <Input aria-label="Quota reserve" type="number" min={0} max={90} className="mt-1 w-28" value={reserve} onChange={event => setReserve(Number(event.target.value))}/>
      </label>
      <p className="text-caption text-muted-foreground">Workflows wait while a provider is above {100 - reserve}% of any usage window.</p>
      <ErrorNote message={error}/>
    </DialogPanel>
    <DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button onClick={() => void save()}>Save</Button></DialogFooter>
  </DialogContent>}</Dialog>;
}
