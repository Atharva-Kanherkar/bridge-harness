import { useMemo, useState } from "react";
import { clsx } from "clsx";
import { ChevronDown, FileCode2, FileDiff, Folder } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { RowButton } from "@/components/ui/row";
import { Tip } from "@/components/ui/tooltip";
import { formatShortcut } from "@/lib/platform";
import { dockPaneChord } from "../dockLayout";
import { shortcutFor } from "../keymap";
import { changeTree, fileName, patchBody, type ChangeMark, type TurnChanges, type TurnEnd, type TurnFileChange } from "../turnChanges";
import { PatchView } from "./DiffView";

/// A turn that touched many files would otherwise stand a hundred rows in the
/// middle of the transcript. Past this the card shows the first files and a
/// way to the rest.
const SHOWN_FILES = 10;

const MARK_TONE: Record<ChangeMark, string> = { A: "text-success", M: "text-info", D: "text-destructive" };
const MARK_WORD: Record<ChangeMark, string> = { A: "added", M: "modified", D: "deleted" };

/** `+12 −3`, green then red, in the mono figures every diffstat in Bridge uses. */
function DiffCounts({ additions, deletions }: { additions: number; deletions: number }) {
  return <span className="inline-flex shrink-0 gap-1.5 font-mono text-caption tabular-nums">
    <span className="text-success">+{additions}</span>
    <span className="text-destructive">−{deletions}</span>
  </span>;
}

/** The chord the Changes pane opens on, from the dock's own pane order. */
function changesChord(): string | null {
  const digit = dockPaneChord("changes");
  if (!digit) return null;
  const { meta, alt, shift } = shortcutFor("open-dock-pane").chord;
  return formatShortcut({ meta, alt, shift }, digit);
}

/**
 * What a settled turn changed, as the turn's last word: a header with the
 * totals, then the files as a folder tree with their mark and counts. A file
 * row opens the diffs this turn applied to it; Review hands off to the
 * Changes pane, where the working tree as a whole is reviewed.
 */
export function TurnChangesCard({ changes, onReview, className }: { changes: TurnChanges; onReview?: () => void; className?: string }) {
  const [openFiles, setOpenFiles] = useState<ReadonlySet<string>>(() => new Set());
  const [closedFolders, setClosedFolders] = useState<ReadonlySet<string>>(() => new Set());
  const [showAll, setShowAll] = useState(false);
  const tree = useMemo(() => changeTree(changes.files), [changes.files]);
  const expandable = useMemo(() => changes.files.filter(file => file.patches.length > 0), [changes.files]);
  const allOpen = expandable.length > 0 && expandable.every(file => openFiles.has(file.path));
  const count = changes.files.length;
  const chord = onReview ? changesChord() : null;
  const hidden = showAll ? 0 : Math.max(0, count - SHOWN_FILES);

  // The first files in tree order, so a cut never leaves a folder half drawn
  // above a file from a folder further down.
  const shown: typeof tree = [];
  let budget = showAll ? Infinity : SHOWN_FILES;
  for (const group of tree) {
    if (budget <= 0) break;
    const files = group.files.slice(0, budget);
    budget -= files.length;
    shown.push({ ...group, files });
  }

  const toggleFile = (path: string) => setOpenFiles(previous => {
    const next = new Set(previous);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    return next;
  });
  const toggleFolder = (folder: string) => setClosedFolders(previous => {
    const next = new Set(previous);
    if (next.has(folder)) next.delete(folder);
    else next.add(folder);
    return next;
  });
  const toggleAll = () => {
    if (allOpen) {
      setOpenFiles(new Set());
      return;
    }
    setOpenFiles(new Set(expandable.map(file => file.path)));
    setClosedFolders(new Set());
    setShowAll(true);
  };

  return <section data-turn-changes aria-label={`${count} ${count === 1 ? "file" : "files"} changed in this turn`} className={clsx("min-w-0 overflow-hidden rounded-xl border border-border bg-card", className)}>
    {/* Wraps: in a narrow conversation the count and the two actions do not
        fit one row, and without it Review was pushed out of the card. */}
    <header className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border px-3.5 py-2">
      <FileDiff size={14} className="shrink-0 text-muted-foreground" aria-hidden="true"/>
      <span className="shrink-0 text-ui font-medium text-foreground">{count} {count === 1 ? "file" : "files"} changed</span>
      <DiffCounts additions={changes.additions} deletions={changes.deletions}/>
      <span className="min-w-0 truncate text-caption text-faint">in this turn</span>
      <span className="ml-auto flex shrink-0 items-center gap-1">
        {expandable.length > 0 && <Button
          variant="text"
          size="xs"
          data-expand-all
          aria-expanded={allOpen}
          onClick={toggleAll}
        >
          <ChevronDown className={clsx("size-3 transition-transform", allOpen && "rotate-180")} aria-hidden="true"/>
          {allOpen ? "Collapse all" : "Expand all"}
        </Button>}
        {onReview && <Button
          variant="outline"
          size="xs"
          data-review-changes
          onClick={onReview}
          tooltip={chord ? `Open the Changes pane  ${chord}` : "Open the Changes pane"}
        >
          Review
          {chord && <Kbd>{chord}</Kbd>}
        </Button>}
      </span>
    </header>
    <div className="px-2 py-1.5">
      {shown.map(group => {
        const closed = closedFolders.has(group.folder);
        return <div key={group.folder || "."} role="group" aria-label={group.folder || "Working directory"}>
          {group.folder && <RowButton
            size="sm"
            tone="plain"
            aria-expanded={!closed}
            onClick={() => toggleFolder(group.folder)}
          >
            <ChevronDown size={12} className={clsx("shrink-0 text-muted-foreground/60 transition-transform", closed && "-rotate-90")} aria-hidden="true"/>
            <Folder size={13} className="shrink-0" aria-hidden="true"/>
            <span className="truncate font-mono">{group.folder}</span>
          </RowButton>}
          {!closed && group.files.map(file => <FileRow
            key={file.path}
            file={file}
            nested={!!group.folder}
            open={openFiles.has(file.path)}
            onToggle={() => toggleFile(file.path)}
          />)}
        </div>;
      })}
      {hidden > 0 && <RowButton
        size="sm"
        tone="muted"
        onClick={() => setShowAll(true)}
      >Show {hidden} more {hidden === 1 ? "file" : "files"}</RowButton>}
    </div>
  </section>;
}

function FileRow({ file, nested, open, onToggle }: { file: TurnFileChange; nested: boolean; open: boolean; onToggle: () => void }) {
  const expandable = file.patches.length > 0;
  const body = <>
    {nested && <span aria-hidden="true" className="w-4 shrink-0"/>}
    <span className={clsx("w-3 shrink-0 font-mono text-caption", MARK_TONE[file.mark])}>{file.mark}</span>
    <FileCode2 size={13} className="shrink-0 text-muted-foreground" aria-hidden="true"/>
    <span className="min-w-0 truncate font-mono text-meta text-foreground">{fileName(file.path)}</span>
    {file.additions !== undefined && <span className="ml-auto pl-2"><DiffCounts additions={file.additions} deletions={file.deletions ?? 0}/></span>}
  </>;
  const hint = `${file.path} · ${MARK_WORD[file.mark]}`;
  return <div data-turn-file={file.path}>
    {expandable
      ? <Tip label={hint}><RowButton size="sm" tone="muted" aria-expanded={open} onClick={onToggle}>{body}</RowButton></Tip>
      : <Tip label={hint}><div className="flex min-h-7 w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-0.5 text-left">{body}</div></Tip>}
    {expandable && open && <div className={clsx("my-1 overflow-hidden rounded-lg border border-border/60", nested ? "ml-7" : "ml-1.5")}>
      <PatchView patch={file.patches.map(patchBody).join("\n")} path={file.path} className="max-h-[360px]" foldAfterHunks={2}/>
    </div>}
  </div>;
}

/** The caption that closes a settled turn: when, how long, which model. */
export function TurnLine({ line, className }: { line: string; className?: string }) {
  return <span data-turn-line className={clsx("text-caption tabular-nums text-faint", className)}>{line}</span>;
}

/**
 * A turn's end drawn on its own, under a row that is not a reply: a tool run
 * or a card. A reply draws the same two pieces inside itself, beside its
 * actions.
 */
export function TurnEndBlock({ end, onReview }: { end: TurnEnd; onReview?: () => void }) {
  return <div data-turn-end className="mt-3 grid min-w-0 gap-1.5">
    {end.changes && <TurnChangesCard changes={end.changes} onReview={onReview}/>}
    {end.line && <TurnLine line={end.line}/>}
  </div>;
}
