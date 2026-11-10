// A page's history, laid out like Notion's: the versions on the right, the
// selected one on the left as the change a restore would make (or read as a
// page), and one button to bring it back. Restoring writes the old words as a
// new change, so nothing in the history is ever lost, including the restore.

import { useEffect, useMemo, useRef, useState } from "react";
import { SkeletonRegion, SkeletonText } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import { historyEntries, lineDiff, type HistoryEntry } from "../../notesHistory";
import type { NotePage } from "../../types";
import { Dialog, DialogPopup, DialogTitle } from "@/components/ui/dialog";
import { HarnessMark } from "../harnessMarks";
import { Markdown } from "../Markdown";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { Pill, PrimaryButton } from "./NotesUi";

const when = (iso: string) => new Date(iso).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

export function NoteHistoryDialog({ page, open, chatTitle, onClose, onRestore }: {
  page: NotePage;
  open: boolean;
  chatTitle: (sessionId: string) => string | undefined;
  onClose: () => void;
  /** Restore version `seq`; resolves once the page holds its words. */
  onRestore: (seq: number) => Promise<boolean>;
}) {
  const id = page.note.id;
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [version, setVersion] = useState<{ seq: number; markdown: string } | null>(null);
  const [view, setView] = useState<"changes" | "version">("changes");
  const [restoring, setRestoring] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // Chat names only label entries; a renamed chat must not refetch history.
  const names = useRef(chatTitle);
  names.current = chatTitle;

  useEffect(() => {
    if (!open) return;
    let current = true;
    setEntries(null);
    setFailure(null);
    bridgeApi.notePageHistory(id).then(history => {
      if (!current) return;
      const next = historyEntries(history.versions, sessionId => names.current(sessionId));
      setEntries(next);
      // The newest entry is the page as it is; the one before is what a
      // person usually came here for.
      setSelected(next[1]?.seq ?? next[0]?.seq ?? null);
    }, error => { if (current) setFailure(error instanceof Error ? error.message : String(error)); });
    return () => { current = false; };
  }, [open, id]);

  useEffect(() => {
    if (!open || selected === null) return;
    let current = true;
    bridgeApi.readNoteVersion(id, selected).then(read => { if (current) setVersion({ seq: read.seq, markdown: read.markdown }); }, error => { if (current) setFailure(error instanceof Error ? error.message : String(error)); });
    return () => { current = false; };
  }, [open, id, selected]);

  const shown = version && version.seq === selected ? version : null;
  const rows = useMemo(() => shown ? lineDiff(page.markdown, shown.markdown) : [], [shown, page.markdown]);
  const same = shown?.markdown === page.markdown;

  const restore = async () => {
    if (selected === null) return;
    setRestoring(true);
    try { if (await onRestore(selected)) onClose(); } finally { setRestoring(false); }
  };

  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>
    <DialogPopup aria-label="Page history" className="h-[min(80vh,46rem)] max-w-5xl overflow-hidden p-0" showCloseButton={false}>
      <div className="flex min-h-0 flex-1">
        <section aria-label="Selected version" className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center gap-2 px-6">
            <DialogTitle className="m-0 flex-1 truncate text-[15px] font-semibold text-foreground">Page history</DialogTitle>
            <div role="tablist" aria-label="Show" className="inline-flex rounded-full bg-muted p-0.5">
              {([["changes", "Changes"], ["version", "Version"]] as const).map(([key, label]) =>
                <button key={key} type="button" role="tab" aria-selected={view === key} onClick={() => setView(key)} className={clsx("h-7 rounded-full px-3 text-[12.5px] transition-colors", view === key ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:text-foreground")}>{label}</button>)}
            </div>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
            {failure && <p role="alert" className="m-0 text-[13px] text-destructive">{failure}</p>}
            {!failure && !shown && <SkeletonRegion label="Loading version"><SkeletonText lines={6} /></SkeletonRegion>}
            {shown && view === "version" && <div className="note-md"><Markdown text={shown.markdown} /></div>}
            {shown && view === "changes" && (same
              ? <p className="m-0 text-[13px] text-muted-foreground">This is the page as it is now.</p>
              : <>
                <p className="m-0 mb-3 text-[12px] text-muted-foreground">Restoring brings back the marked + lines and removes the − lines. Status, project and tags stay as they are.</p>
                <ol aria-label="Changes" className="m-0 list-none overflow-hidden rounded-xl p-0 font-mono text-[12.5px] leading-6 ring-1 ring-border">
                  {rows.map((row, index) => <li key={index} data-diff={row.kind} className={clsx("flex gap-3 whitespace-pre-wrap px-3", row.kind === "added" && "bg-success/10", row.kind === "removed" && "bg-destructive/10 text-muted-foreground line-through decoration-destructive/40")}>
                    <span aria-hidden="true" className="w-3 shrink-0 select-none text-faint">{row.kind === "added" ? "+" : row.kind === "removed" ? "−" : ""}</span>
                    <span className="min-w-0 flex-1">{row.text || " "}</span>
                  </li>)}
                </ol>
              </>)}
          </div>
        </section>

        <aside aria-label="Versions" className="flex w-72 shrink-0 flex-col border-l border-border bg-sidebar">
          <ul aria-label="Versions list" className="m-0 min-h-0 flex-1 list-none space-y-px overflow-y-auto p-2">
            {entries?.map((entry, index) => {
              const harness = /^agent:([^:]+):/.exec(entry.actor)?.[1];
              return <li key={entry.seq}>
                <button type="button" aria-current={entry.seq === selected ? "true" : undefined} onClick={() => setSelected(entry.seq)} className={clsx("block w-full rounded-lg px-3 py-2 text-left transition-colors", entry.seq === selected ? "bg-selection" : "hover:bg-accent")}>
                  <span className="flex items-center gap-1.5 text-[13px] text-foreground">
                    {harness && <HarnessMark harness={harness} size={12} />}
                    <span className="min-w-0 flex-1 truncate">{entry.label}</span>
                    {index === 0 && <span className="shrink-0 text-[11px] text-faint">Current</span>}
                  </span>
                  <span className="mt-0.5 flex items-center gap-2 text-[11.5px] text-muted-foreground">
                    <span>{when(entry.at)}</span>
                    <span className="tabular-nums text-faint">+{entry.added} −{entry.removed}</span>
                    {entry.count > 1 && <span className="text-faint">{entry.count} edits</span>}
                  </span>
                </button>
              </li>;
            })}
            {entries && !entries.length && <li className="px-3 py-6 text-center text-[12.5px] text-muted-foreground">No versions yet.</li>}
          </ul>
          <div className="flex shrink-0 items-center justify-end gap-1 p-3">
            <Pill onClick={onClose}>Cancel</Pill>
            <PrimaryButton disabled={!shown || same || restoring} onClick={() => void restore()}>{restoring ? "Restoring…" : "Restore this version"}</PrimaryButton>
          </div>
        </aside>
      </div>
    </DialogPopup>
  </Dialog>;
}
