// The Notes screen before its data arrives: every placeholder is the shape of
// what replaces it, at the same place and size, so a page or a list fills in
// rather than jumping. Kept free of the editor and the database code, so App
// can draw `NotesScreenSkeleton` while the Notes chunk itself still loads.

import { clsx } from "clsx";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";

const LIST_OPEN_KEY = "bridge.notes.listOpen";
/** Whether the notes list is shown beside the page (`toggle-notes-list`). */
export const readListOpen = () => { try { return localStorage.getItem(LIST_OPEN_KEY) !== "0"; } catch { return true; } };
/** Remember the list's visibility for the next visit. */
export const writeListOpen = (open: boolean) => { try { localStorage.setItem(LIST_OPEN_KEY, open ? "1" : "0"); } catch { /* storage off */ } };

// Ragged, like real titles and first lines; fixed so a re-render never reshuffles them.
const ROW_TITLES = ["w-3/5", "w-2/5", "w-1/2", "w-3/4", "w-2/5", "w-2/3"] as const;
const ROW_LINES = ["w-4/5", "w-3/5", "w-11/12", "w-1/2", "w-3/4", "w-2/3"] as const;
const PROPERTY_VALUES = ["w-20", "w-14", "w-28", "w-24"] as const;
const TAB_WIDTHS = ["w-8", "w-11", "w-11", "w-10"] as const;

/** Rows shaped like the list's `NoteRow`: a title, its age, a first line. */
export function NoteListSkeleton({ rows = 6, className }: { rows?: number; className?: string }) {
  return <SkeletonRegion label="Loading notes" className={clsx("space-y-px", className)}>
    {Array.from({ length: rows }, (_, row) => <div key={row} data-skeleton-row="" className="flex items-stretch">
      <span aria-hidden="true" className="w-5 shrink-0" />
      <div className="min-w-0 flex-1 px-2.5 py-2">
        <span className="flex h-5 items-center gap-3">
          <Skeleton index={row} className={clsx("h-2.5", ROW_TITLES[row % ROW_TITLES.length])} />
          <Skeleton index={row} className="ml-auto h-2 w-5 shrink-0" />
        </span>
        <span className="mt-0.5 flex h-[1.1875rem] items-center">
          <Skeleton index={row} className={clsx("h-2", ROW_LINES[row % ROW_LINES.length])} />
        </span>
      </div>
    </div>)}
  </SkeletonRegion>;
}

/** The tab chips (All, Inbox, Pages, …) before the system database is read. */
export function NoteTabsSkeleton({ className }: { className?: string }) {
  return <span aria-hidden="true" className={clsx("flex gap-0.5", className)}>
    {TAB_WIDTHS.map((width, tab) => <span key={tab} className="flex h-7 items-center px-2"><Skeleton index={tab} className={clsx("h-2.5", width)} /></span>)}
  </span>;
}

/**
 * The open page before its read lands, laid out like `NotePageView`'s article:
 * the title (in words when the list already knows it), the properties, the
 * view switch with its actions, then the prose.
 */
export function NotePageSkeleton({ title }: { title?: string | null }) {
  return <SkeletonRegion label={title ? `Opening ${title}` : "Opening the page"} className="mx-auto w-full max-w-[44rem] flex-1 px-8 pb-24 pt-10">
    {title
      ? <p aria-hidden="true" className="m-0 break-words text-[2.25rem] font-bold leading-[1.2] tracking-[-0.025em] text-foreground">{title}</p>
      : <span className="flex h-[2.7rem] items-center"><Skeleton className="h-7 w-3/5 rounded-md" /></span>}
    <div className="mt-5">
      {PROPERTY_VALUES.map((width, row) => <div key={row} className="flex h-8 items-center gap-2">
        <span className="w-28 shrink-0"><Skeleton index={row} className="h-2.5 w-14" /></span>
        <Skeleton index={row} className={clsx("h-2.5", width)} />
      </div>)}
    </div>
    <div className="mt-6 flex items-center gap-2 border-b border-border pb-3">
      <Skeleton index={3} className="mr-auto h-8 w-[11.5rem] rounded-full" />
      {["w-16", "w-24", "w-20"].map((width, action) => <span key={action} className="hidden h-8 items-center px-3 sm:flex"><Skeleton index={3} className={clsx("h-2.5", width)} /></span>)}
    </div>
    <div className="mt-6 flex flex-col gap-3.5">
      {["w-full", "w-11/12", "w-full", "w-2/5"].map((width, line) => <Skeleton key={line} index={4 + Math.floor(line / 2)} className={width} />)}
    </div>
    <div className="mt-7 flex flex-col gap-3.5">
      {["w-10/12", "w-full", "w-3/5"].map((width, line) => <Skeleton key={line} index={6 + Math.floor(line / 2)} className={width} />)}
    </div>
  </SkeletonRegion>;
}

/**
 * The whole screen while the Notes code loads, so the first visit opens on
 * the list's outline rather than a line of text in an empty pane.
 */
export function NotesScreenSkeleton() {
  return <div className="flex h-full min-h-0 bg-background">
    <aside aria-label="Notes list" className={clsx("flex min-h-0 w-full shrink-0 flex-col border-r border-border md:w-[18.5rem]", readListOpen() ? "md:flex" : "md:hidden")}>
      <div className="flex h-12 shrink-0 items-center pl-4 pr-2">
        <h2 className="m-0 flex-1 text-[13px] font-semibold text-foreground">Notes</h2>
      </div>
      <div aria-hidden="true" className="shrink-0 px-3 pt-3">
        <div className="h-[5.25rem] rounded-2xl bg-card shadow-sm ring-1 ring-border" />
      </div>
      <div aria-hidden="true" className="shrink-0 px-3 pt-3">
        <div className="h-8 rounded-lg bg-muted" />
        <NoteTabsSkeleton className="mt-2" />
      </div>
      <NoteListSkeleton className="min-h-0 flex-1 overflow-hidden px-2 py-2" />
    </aside>
    <main className="hidden min-w-0 flex-1 md:block" />
  </div>;
}
