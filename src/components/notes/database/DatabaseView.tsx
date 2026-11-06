// A database's views over `notes/query_database`: the view tabs, and the
// active view (a table here; board, calendar and list arrive later).
//
// The server filters and sorts. Every write re-queries, and so does every
// `notes/page_changed`, coalesced; a newer query always wins over an older
// one. A committed cell shows at once and holds until a query that started
// after its write lands, or goes back when the write is refused.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CalendarDays, Kanban, List, Table2 } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { bridgeApi } from "../../../api";
import type { DatabaseRow, NoteDatabase, NoteDatabaseView, NoteProperty, NoteSchemaChange, NoteViewInput, Workspace } from "../../../types";
import { DatabaseTable, type TableActions } from "./DatabaseTable";
import type { CellHost } from "./PropertyCell";
import { columnsOf, viewInputOf, withColumn, type Column } from "./formatValue";
import { addOption, saveOptions } from "./schemaWrites";

export const FIRST_PAGE = 100;
export const LATER_PAGE = 500;
const COALESCE_MS = 30;

const VIEW_ICONS: Record<string, typeof Table2> = { table: Table2, board: Kanban, calendar: CalendarDays, list: List };

export interface DatabaseViewProps {
  databaseId: string;
  /** The database as the page holding it last read it; read here when absent. */
  database?: NoteDatabase | null;
  viewId?: string | null;
  /** A bounded frame inside another page. */
  bounded?: boolean;
  /** The page holding an embed: its own saves change no row, so they read nothing. */
  hostPageId?: string;
  workspaces: Workspace[];
  chatTitle?: (sessionId: string) => string | undefined;
  onOpenChat?: (sessionId: string) => void;
  onOpenPage: (id: string) => void;
  onError: (message: string) => void;
}

interface Pending { row: string; key: string; value: unknown; after: number }

const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const withView = (database: NoteDatabase, view: NoteDatabaseView): NoteDatabase => ({ ...database, views: database.views.map(candidate => candidate.id === view.id ? view : candidate) });

export function DatabaseView({ databaseId, database: given, viewId, bounded = false, hostPageId, workspaces, chatTitle, onOpenChat, onOpenPage, onError }: DatabaseViewProps) {
  const [database, setDatabase] = useState<NoteDatabase | null>(given ?? null);
  const [activeId, setActiveId] = useState<string | null>(viewId ?? null);
  const [rows, setRows] = useState<DatabaseRow[] | null>(null);
  const [pending, setPending] = useState<Pending[]>([]);
  const [related, setRelated] = useState<Map<string, string>>(new Map());
  const ticket = useRef(0);
  const painted = useRef(false);
  /** Rows of the current view have landed once. */
  const loaded = useRef(false);
  const latest = useRef({ onError });
  latest.current = { onError };
  const fail = useCallback((error: unknown) => latest.current.onError(message(error)), []);

  /** The newest view edit not yet saved, kept over any read that lands first. */
  const viewDraft = useRef<NoteDatabaseView | null>(null);
  useEffect(() => { if (given) setDatabase(viewDraft.current ? withView(given, viewDraft.current) : given); }, [given]);

  const readDatabase = useCallback(async () => {
    try {
      const page = await bridgeApi.readNotePage(databaseId);
      if (page.database) setDatabase(viewDraft.current ? withView(page.database, viewDraft.current) : page.database);
    } catch (error) { fail(error); }
  }, [databaseId, fail]);
  useEffect(() => { if (!given) void readDatabase(); }, [given, readDatabase]);

  const views = useMemo(() => [...(database?.views ?? [])].sort((a, b) => a.order - b.order), [database]);
  const view: NoteDatabaseView | undefined = views.find(candidate => candidate.id === activeId) ?? views[0];
  const queryViewId = view?.id ?? null;

  /** The rows, the first page at once and the rest by cursor behind it. */
  const query = useCallback(async () => {
    const mine = ++ticket.current;
    if (!painted.current) performance.mark?.("notes.table.query-start");
    try {
      // The first load paints its first page at once. A refresh replaces
      // nothing until it holds every page, so the table never shrinks under
      // a scrolled view or an open editor.
      const refresh = loaded.current;
      let result = await bridgeApi.queryDatabase({ databaseId, viewId: queryViewId, limit: FIRST_PAGE });
      if (mine !== ticket.current) return;
      let all = result.rows;
      const land = () => {
        setRows(all);
        setPending(current => current.filter(item => item.after >= mine));
        loaded.current = true;
      };
      if (!refresh) land();
      while (result.nextCursor) {
        result = await bridgeApi.queryDatabase({ databaseId, viewId: queryViewId, limit: LATER_PAGE, cursor: result.nextCursor });
        if (mine !== ticket.current) return;
        all = [...all, ...result.rows];
        if (!refresh) setRows(all);
      }
      if (refresh) land();
    } catch (error) {
      if (mine === ticket.current) fail(error);
    }
  }, [databaseId, queryViewId, fail]);

  useEffect(() => { loaded.current = false; if (database) void query(); }, [database ? database.id : null, queryViewId, query]); // eslint-disable-line react-hooks/exhaustive-deps

  // The first painted row closes the first-paint measure.
  useEffect(() => {
    if (painted.current || !rows) return;
    painted.current = true;
    try { performance.mark?.("notes.table.painted"); performance.measure?.("notes.table.first-paint", "notes.table.query-start", "notes.table.painted"); } catch { /* no marks */ }
  }, [rows]);

  // Anyone's change re-reads, a burst as one read.
  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    let burst: ReturnType<typeof setTimeout> | undefined;
    // Whether any event in the burst named the database, not just the last:
    // a retype writes the database, then each row it converts.
    let schemaChanged = false;
    void bridgeApi.onNotesPageChanged(payload => {
      if (payload.pageId === hostPageId) return;
      if (payload.pageId === databaseId) schemaChanged = true;
      clearTimeout(burst);
      burst = setTimeout(() => {
        if (schemaChanged && !given) void readDatabase();
        schemaChanged = false;
        void query();
      }, COALESCE_MS);
    }).then(unlisten => { if (alive) stop = unlisten; else unlisten(); });
    return () => { alive = false; clearTimeout(burst); stop?.(); };
  }, [databaseId, given, hostPageId, readDatabase, query]);

  // Titles of related rows, read once per related database.
  const relationTargets = useMemo(() => [...new Set((database?.properties ?? []).map(property => property.relation?.databaseId).filter((id): id is string => !!id))].sort().join(" "), [database]);
  useEffect(() => {
    if (!relationTargets) return;
    let alive = true;
    void Promise.all(relationTargets.split(" ").map(id => bridgeApi.queryDatabase({ databaseId: id, limit: LATER_PAGE }).then(result => result.rows, () => [] as DatabaseRow[])))
      .then(lists => { if (alive) setRelated(new Map(lists.flat().map(row => [row.note.id, row.note.title ?? ""]))); });
    return () => { alive = false; };
  }, [relationTargets]);

  const shown = useMemo(() => (rows ?? []).map(row => {
    const mine = pending.filter(item => item.row === row.note.id);
    if (!mine.length) return row;
    const note = { ...row.note };
    const values = { ...row.values };
    for (const item of mine) {
      if (item.key === "title") note.title = item.value as string;
      else if (item.key === "project") note.workspaceId = item.value as string | null;
      else if (item.value === null) delete values[item.key];
      else values[item.key] = item.value;
    }
    return { note, values };
  }), [rows, pending]);

  const commit = useCallback(async (row: DatabaseRow, column: Column, value: unknown) => {
    const entry: Pending = { row: row.note.id, key: column.id, value, after: Number.POSITIVE_INFINITY };
    setPending(current => [...current.filter(item => !(item.row === entry.row && item.key === entry.key)), entry]);
    try {
      if (column.id === "title") await bridgeApi.updateNote({ id: row.note.id, title: String(value ?? "") });
      else if (column.id === "project") await bridgeApi.updateNote({ id: row.note.id, ...(value ? { workspaceId: String(value) } : { clearWorkspace: true }) });
      else await bridgeApi.updateNote({ id: row.note.id, values: { [column.id]: value ?? null } });
      // Held until a query that starts from here lands.
      entry.after = ticket.current;
      setPending(current => [...current]);
      void query();
    } catch (error) {
      setPending(current => current.filter(item => item !== entry));
      fail(error);
    }
  }, [query, fail]);

  const change = useCallback(async (changes: NoteSchemaChange[]): Promise<NoteDatabase | null> => {
    try {
      const next = await bridgeApi.updateDatabaseSchema({ id: databaseId, changes });
      setDatabase(next);
      void query();
      return next;
    } catch (error) { fail(error); return null; }
  }, [databaseId, query, fail]);

  // View edits apply at once, each built on the one before it (still unsaved
  // or not), and save in order; only the newest answer replaces the view.
  const latestDatabase = useRef(database);
  latestDatabase.current = database;
  const latestView = useRef(view);
  latestView.current = view;
  const saves = useRef<Promise<unknown>>(Promise.resolve());
  const saveSeq = useRef(0);
  const saveView = useCallback((patch: (base: NoteDatabaseView, columns: Column[]) => Partial<NoteViewInput>) => {
    const current = latestDatabase.current;
    const shown = latestView.current;
    if (!current || !shown) return;
    const base = viewDraft.current?.id === shown.id ? viewDraft.current : shown;
    const input: NoteViewInput = { ...viewInputOf(base), ...patch(base, columnsOf(current, base)) };
    const optimistic: NoteDatabaseView = {
      id: base.id, name: input.name, type: input.type, order: input.order ?? base.order, filter: input.filter,
      sorts: input.sorts ?? [], groupBy: input.groupBy ?? null, dateProperty: input.dateProperty ?? null, properties: input.properties ?? [],
    };
    viewDraft.current = optimistic;
    setDatabase(held => held && withView(held, optimistic));
    const mine = ++saveSeq.current;
    saves.current = saves.current.then(() => bridgeApi.saveDatabaseView({ databaseId, view: input })).then(saved => {
      if (mine !== saveSeq.current) return;
      viewDraft.current = null;
      setDatabase(held => held && withView(held, saved));
      void query();
    }, error => {
      if (mine === saveSeq.current) { viewDraft.current = null; void readDatabase(); }
      fail(error);
    });
  }, [databaseId, query, readDatabase, fail]);

  const host: CellHost = useMemo(() => ({
    addOption: async (property: NoteProperty, option) => {
      try {
        const next = await addOption(databaseId, property.id, option);
        setDatabase(next);
        void query();
        return next.properties.find(candidate => candidate.id === property.id) ?? null;
      } catch (error) { fail(error); return null; }
    },
    searchRows: async (target, text) => (await bridgeApi.queryDatabase({ databaseId: target, query: text.trim() || null, limit: 20 })).rows.map(row => ({ id: row.note.id, title: row.note.title ?? "" })),
    rowTitle: id => related.get(id) ?? rows?.find(row => row.note.id === id)?.note.title ?? undefined,
    workspaces,
    chatTitle,
    onOpenChat,
  }), [databaseId, query, fail, related, rows, workspaces, chatTitle, onOpenChat]);

  const actions: TableActions = useMemo(() => ({
    rename: (property, name) => void change([{ op: "rename", propertyId: property.id, name }]),
    retype: (property, type) => void change([{ op: "retype", propertyId: property.id, type }]),
    setOptions: (property, options, seen) => void saveOptions(databaseId, property.id, options, seen).then(next => { setDatabase(next); void query(); }, fail),
    remove: property => void change([{ op: "delete", propertyId: property.id }]),
    addProperty: (name, type) => void change([{ op: "add", property: { name, type, ...(type === "status" ? { options: [{ name: "To do", group: "todo" }, { name: "In progress", group: "inProgress" }, { name: "Done", group: "done" }] } : {}) } }]),
    sort: (column, direction) => saveView(() => ({ sorts: [{ property: column.id, direction }] })),
    hide: column => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { visible: false }) })),
    show: column => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { visible: true }) })),
    resize: (column, width) => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { width }) })),
  }), [databaseId, change, saveView, query, fail]);

  const newRow = useCallback(async () => {
    try {
      const note = await bridgeApi.createNotePage({ title: null, content: "", parentId: databaseId });
      void query();
      return note.id;
    } catch (error) { fail(error); return null; }
  }, [databaseId, query, fail]);

  if (!database) return <p role="status" className="m-0 py-3 text-[13px] text-muted-foreground">Loading the database…</p>;

  return <section aria-label="Database" className="min-w-0">
    {views.length > 1 && <div role="tablist" aria-label="Views" className="mb-2 flex flex-wrap gap-0.5">
      {views.map(candidate => {
        const Icon = VIEW_ICONS[candidate.type] ?? Table2;
        return <button key={candidate.id} type="button" role="tab" aria-selected={candidate.id === view?.id} onClick={() => setActiveId(candidate.id)} className={clsx("inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12.5px] transition-colors", candidate.id === view?.id ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}>
          <Icon size={13} strokeWidth={1.75} aria-hidden="true" />{candidate.name}
        </button>;
      })}
    </div>}
    {view && view.type !== "table"
      ? <p role="status" className="m-0 rounded-lg bg-muted px-3 py-6 text-center text-[13px] text-muted-foreground">Board, calendar and list views arrive in a later update.</p>
      : rows === null
        ? <p role="status" className="m-0 py-3 text-[13px] text-muted-foreground">Loading rows…</p>
        : <DatabaseTable database={database} view={view} rows={shown} host={host} actions={actions} onCommit={(row, column, value) => void commit(row, column, value)} onNewRow={newRow} onOpenRow={onOpenPage} bounded={bounded} />}
  </section>;
}
