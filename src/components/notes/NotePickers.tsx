// The two choices Notes asks for before it hands work to an agent: which
// harness runs a Structure or Turn into tasks pass, and where a chat started
// from a note goes. Each is a popover on the action that needs it.

import { useState, type ReactNode } from "react";
import { Spinner } from "@/components/ui/spinner";
import type { AdapterDescriptor, Workspace } from "../../types";
import { harnessLabel } from "../../utils";
import { HarnessMark } from "../harnessMarks";
import { MenuDivider, MenuLabel, Option, OptionList, Pill, Popover, PrimaryButton } from "./NotesUi";

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

function HarnessOptions({ label, choices, value, defaultHarness, onChange }: { label: string; choices: AdapterDescriptor[]; value: string; defaultHarness: string | null; onChange: (id: string) => void }) {
  return <OptionList label={label}>
    {choices.map(adapter => <Option
      key={adapter.id}
      selected={value === adapter.id}
      onSelect={() => onChange(adapter.id)}
      leading={<HarnessMark harness={adapter.id} size={14} />}
      hint={adapter.id === defaultHarness ? "Default" : undefined}
    >{harnessLabel(adapter.id)}</Option>)}
  </OptionList>;
}

/** A pill that opens a popover; `children` renders inside it with a close. */
export function PopoverAction({ label, triggerLabel, busyLabel, busy = false, panelLabel, align, width, children }: {
  label: ReactNode;
  /** The trigger's accessible name, when its text alone is ambiguous. */
  triggerLabel?: string;
  busyLabel?: string;
  busy?: boolean;
  panelLabel: string;
  align?: "start" | "end";
  width?: string;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  return <div className="relative">
    <Pill aria-label={triggerLabel} aria-expanded={open} aria-haspopup="dialog" active={open} disabled={busy} onClick={() => setOpen(current => !current)}>
      {busy && <Spinner className="size-3.5" />}{busy ? busyLabel : label}
    </Pill>
    <Popover open={open} onClose={close} label={panelLabel} align={align} width={width}>{children(close)}</Popover>
  </div>;
}

export function PassPicker({ label, what, adapters, defaultHarness, onRun }: {
  /** The action's name, used for the Run button. */
  label: string;
  /** One line on what the pass does. */
  what: string;
  adapters: AdapterDescriptor[];
  defaultHarness: string | null;
  onRun: (harness: string) => void;
}) {
  const choices = passHarnesses(adapters);
  const [harness, setHarness] = useState(() => preselect(choices, defaultHarness));
  return <>
    <p className="m-0 px-2.5 pb-1 pt-1.5 text-[12px] leading-relaxed text-muted-foreground">{what}</p>
    {choices.length === 0
      ? <p role="status" className="m-0 px-2.5 pb-2 pt-1 text-[13px] text-foreground">{NO_PASS_HARNESS}</p>
      : <>
        <MenuDivider />
        <MenuLabel>Run on</MenuLabel>
        <HarnessOptions label="Harness" choices={choices} value={harness} defaultHarness={defaultHarness} onChange={setHarness} />
        <div className="flex justify-end px-1 pb-0.5 pt-2"><PrimaryButton onClick={() => onRun(harness)}>{label}</PrimaryButton></div>
      </>}
  </>;
}

export interface StartChoice { workspaceId: string | null; harness: string }

export function StartPicker({ subject = "task", adapters, workspaces, defaultHarness, suggestedWorkspaceId, busy = false, onStart }: {
  /** What goes with the chat as its first message. */
  subject?: "task" | "note";
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  defaultHarness: string | null;
  /** The task's project, else its page's. "No project" when neither. */
  suggestedWorkspaceId: string | null;
  busy?: boolean;
  onStart: (choice: StartChoice) => void;
}) {
  const choices = chatHarnesses(adapters);
  const known = workspaces.some(workspace => workspace.id === suggestedWorkspaceId);
  const [workspaceId, setWorkspaceId] = useState<string | null>(known ? suggestedWorkspaceId : null);
  const [harness, setHarness] = useState(() => preselect(choices, defaultHarness));
  return <>
    <MenuLabel>Where</MenuLabel>
    <OptionList label="Project" className="max-h-44 overflow-y-auto">
      <Option selected={workspaceId === null} onSelect={() => setWorkspaceId(null)} hint="Direct chat">No project</Option>
      {workspaces.map(workspace => <Option key={workspace.id} selected={workspaceId === workspace.id} onSelect={() => setWorkspaceId(workspace.id)}>{workspace.title}</Option>)}
    </OptionList>
    <MenuDivider />
    <MenuLabel>Run on</MenuLabel>
    {choices.length
      ? <HarnessOptions label="Chat harness" choices={choices} value={harness} defaultHarness={defaultHarness} onChange={setHarness} />
      : <p role="status" className="m-0 px-2.5 pb-1 text-[13px] text-foreground">No harness can start a chat: sign in to one in Settings.</p>}
    <div className="flex items-center gap-2 px-2.5 pb-0.5 pt-2.5">
      <p className="m-0 min-w-0 flex-1 text-[11px] leading-snug text-muted-foreground">The {subject} goes with it as the first message.</p>
      <PrimaryButton disabled={busy || !harness} onClick={() => onStart({ workspaceId, harness })}>{busy ? "Starting…" : "Start"}</PrimaryButton>
    </div>
  </>;
}
