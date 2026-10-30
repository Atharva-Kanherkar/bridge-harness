// The two small choices Notes asks for before it hands work to an agent:
// which harness runs a Structure or Turn into tasks pass, and where a chat
// started from a note goes. Both render inline under the control that opened
// them, so nothing is portalled out of the page's scroll.

import { useState } from "react";
import type { AdapterDescriptor, Workspace } from "../../types";
import { harnessLabel } from "../../utils";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

/** Harnesses that can run a note pass: hidden, tool-less generations. */
export const PASS_HARNESSES = ["claude", "codex", "opencode"];
export const NO_PASS_HARNESS = "No harness can run this: sign in to Claude, Codex or OpenCode.";

export function passHarnesses(adapters: AdapterDescriptor[]): AdapterDescriptor[] {
  return PASS_HARNESSES.map(id => adapters.find(adapter => adapter.id === id && adapter.available)).filter((adapter): adapter is AdapterDescriptor => !!adapter);
}

export function chatHarnesses(adapters: AdapterDescriptor[]): AdapterDescriptor[] {
  return adapters.filter(adapter => adapter.available && adapter.id !== "shell");
}

const preselect = (choices: AdapterDescriptor[], preferred: string | null | undefined) =>
  (choices.find(adapter => adapter.id === preferred) ?? choices[0])?.id ?? "";

const PANEL = "mt-2 rounded-xl border border-border bg-card p-3 text-caption shadow-sm";
const SELECT = "h-8 min-w-0 rounded-lg border border-border bg-background px-2 text-ui text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";
const PRIMARY = "inline-flex h-8 items-center rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40";
const QUIET = "inline-flex h-8 items-center rounded-lg px-3 text-ui text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";

export function HarnessPicker({ label, what, adapters, defaultHarness, onRun, onCancel }: {
  /** The action's name, used for the Run button and the panel's label. */
  label: string;
  /** One line on what the pass does. */
  what: string;
  adapters: AdapterDescriptor[];
  defaultHarness: string | null;
  onRun: (harness: string) => void;
  onCancel: () => void;
}) {
  const choices = passHarnesses(adapters);
  const [harness, setHarness] = useState(() => preselect(choices, defaultHarness));
  return <div role="group" aria-label={`${label} options`} className={PANEL}>
    <p className="m-0 text-muted-foreground">{what}</p>
    {choices.length === 0
      ? <div className="mt-2 flex items-center justify-between gap-2">
        <p role="status" className="m-0 text-foreground">{NO_PASS_HARNESS}</p>
        <button type="button" onClick={onCancel} className={QUIET}>Close</button>
      </div>
      : <div className="mt-2 flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2 text-muted-foreground">
          Run on
          <select aria-label="Harness" value={harness} onChange={event => setHarness(event.target.value)} className={SELECT}>
            {choices.map(adapter => <option key={adapter.id} value={adapter.id}>{harnessLabel(adapter.id)}{adapter.id === defaultHarness ? " (default)" : ""}</option>)}
          </select>
        </label>
        <span className="flex-1" />
        <button type="button" onClick={onCancel} className={QUIET}>Cancel</button>
        <button type="button" onClick={() => onRun(harness)} className={PRIMARY}>{label}</button>
      </div>}
  </div>;
}

export interface StartChoice { workspaceId: string | null; harness: string }

export function StartPicker({ subject = "task", adapters, workspaces, defaultHarness, suggestedWorkspaceId, busy = false, onStart, onCancel }: {
  /** What goes with the chat as its first message. */
  subject?: "task" | "note";
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  defaultHarness: string | null;
  /** The task's project, else its page's. "No project" when neither. */
  suggestedWorkspaceId: string | null;
  busy?: boolean;
  onStart: (choice: StartChoice) => void;
  onCancel: () => void;
}) {
  const choices = chatHarnesses(adapters);
  const known = workspaces.some(workspace => workspace.id === suggestedWorkspaceId);
  const [workspaceId, setWorkspaceId] = useState(known ? suggestedWorkspaceId ?? "" : "");
  const [harness, setHarness] = useState(() => preselect(choices, defaultHarness));
  return <div role="group" aria-label="Start a chat" className={PANEL}>
    <div className="flex flex-wrap items-center gap-2">
      <label className="flex min-w-0 items-center gap-2 text-muted-foreground">
        Project
        <select aria-label="Project" value={workspaceId} onChange={event => setWorkspaceId(event.target.value)} className={clsx(SELECT, "max-w-56")}>
          <option value="">No project</option>
          {workspaces.map(workspace => <option key={workspace.id} value={workspace.id}>{workspace.title}</option>)}
        </select>
      </label>
      <label className="flex items-center gap-2 text-muted-foreground">
        Harness
        <select aria-label="Chat harness" value={harness} onChange={event => setHarness(event.target.value)} className={SELECT}>
          {choices.map(adapter => <option key={adapter.id} value={adapter.id}>{harnessLabel(adapter.id)}{adapter.id === defaultHarness ? " (default)" : ""}</option>)}
        </select>
      </label>
      <span className="flex-1" />
      <button type="button" onClick={onCancel} className={QUIET}>Cancel</button>
      <button type="button" disabled={busy || !harness} onClick={() => onStart({ workspaceId: workspaceId || null, harness })} className={PRIMARY}>{busy ? "Starting…" : "Start"}</button>
    </div>
    {choices.length === 0 && <p role="status" className="m-0 mt-2 text-foreground">No harness can start a chat: sign in to one in Settings.</p>}
    <p className="m-0 mt-2 text-muted-foreground">{workspaceId ? "Starts a chat in this project." : "Starts a direct chat with no project."} The {subject} goes with it as the first message.</p>
  </div>;
}
