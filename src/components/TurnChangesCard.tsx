import { useMemo, useState } from "react";
import { clsx } from "clsx";
import { ChevronDown, FileCode2, FileDiff, Folder } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import { formatShortcut } from "@/lib/platform";
import { dockPaneChord } from "../dockLayout";
import { shortcutFor } from "../keymap";
import { changeTree, fileName, type ChangeMark, type TurnChanges, type TurnEnd, type TurnFileChange } from "../turnChanges";
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
  let budget = showAll ? Infinity : SHOWN_FILES;
  const shown = tree.flatMap(group => {
    if (budget <= 0) return [];
    const files = group.files.slice(0, budget);
    budget -= files.length;
    return [{ ...group, files }];
  });

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
    <header className="flex min-w-0 items-center gap-2 border-b border-border px-3.5 py-2">
      <FileDiff size={14} className="shrink-0 text-muted-foreground" aria-hidden="true"/>
      <span className="shrink-0 text-ui font-medium text-foreground">{count} {count === 1 ? "file" : "files"} changed</span>
      <DiffCounts additions={changes.additions} deletions={changes.deletions}/>
      <span className="min-w-0 truncate text-caption text-faint">in this turn</span>
      <span className="ml-auto flex shrink-0 items-center gap-1">
        {expandable.length > 0 && <button
          type="button"
          data-expand-all
          aria-expanded={allOpen}
          onClick={toggleAll}
          className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <ChevronDown size={12} className={clsx("transition-transform", allOpen && "rotate-180")} aria-hidden="true"/>
          {allOpen ? "Collapse all" : "Expand all"}
        </button>}
        {onReview && <button
          type="button"
          data-review-changes
          onClick={onReview}
          title={chord ? `Open the Changes pane  ${chord}` : "Open the Changes pane"}
          className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border bg-background px-2 text-meta text-foreground transition-colors hover:bg-accent"
        >
          Review
          {chord && <Kbd>{chord}</Kbd>}
        </button>}
      </span>
    </header>
    <div className="px-2 py-1.5">
      {shown.map(group => {
        const closed = closedFolders.has(group.folder);
        return <div key={group.folder || "."} role="group" aria-label={group.folder || "Working directory"}>
          {group.folder && <button
            type="button"
            aria-expanded={!closed}
            onClick={() => toggleFolder(group.folder)}
            className="flex h-7 w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 text-left font-mono text-caption text-muted-foreground transition-colors hover:text-foreground"
          >
            <ChevronDown size={12} className={clsx("shrink-0 text-muted-foreground/60 transition-transform", closed && "-rotate-90")} aria-hidden="true"/>
            <Folder size={13} className="shrink-0" aria-hidden="true"/>
            <span className="truncate">{group.folder}</span>
          </button>}
          {!closed && group.files.map(file => <FileRow
            key={file.path}
            file={file}
            nested={!!group.folder}
            open={openFiles.has(file.path)}
            onToggle={() => toggleFile(file.path)}
          />)}
        </div>;
      })}
      {hidden > 0 && <button
        type="button"
        onClick={() => setShowAll(true)}
        className="flex h-7 w-full items-center rounded-md px-1.5 text-left text-meta text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground"
      >Show {hidden} more {hidden === 1 ? "file" : "files"}</button>}
    </div>
  </section>;
}

function FileRow({ file, nested, open, onToggle }: { file: TurnFileChange; nested: boolean; open: boolean; onToggle: () => void }) {
  const expandable = file.patches.length > 0;
  const body = <>
    <span className={clsx("w-3 shrink-0 font-mono text-caption", MARK_TONE[file.mark])} title={MARK_WORD[file.mark]}>{file.mark}</span>
    <FileCode2 size={13} className="shrink-0 text-muted-foreground" aria-hidden="true"/>
    <span className="min-w-0 truncate font-mono text-meta text-foreground">{fileName(file.path)}</span>
    {file.additions !== undefined && <span className="ml-auto pl-2"><DiffCounts additions={file.additions} deletions={file.deletions ?? 0}/></span>}
  </>;
  const row = clsx("flex h-7 w-full min-w-0 items-center gap-2 rounded-md pr-1.5 text-left", nested ? "pl-7" : "pl-1.5");
  return <div data-turn-file={file.path}>
    {expandable
      ? <button type="button" aria-expanded={open} title={file.path} onClick={onToggle} className={clsx(row, "transition-colors hover:bg-accent/50")}>{body}</button>
      : <div title={file.path} className={row}>{body}</div>}
    {expandable && open && <div className={clsx("my-1 overflow-hidden rounded-lg border border-border/60", nested ? "ml-7" : "ml-1.5")}>
      <PatchView patch={file.patches.join("\n")} path={file.path} className="max-h-[360px]" foldAfterHunks={2}/>
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
