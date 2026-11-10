// A database's views over `notes/query_database`: the view tabs, the
// toolbar (an unsaved filter, sort and grouping override), and the active
// view as a table, board, calendar or list.
//
// The server filters and sorts. Every write re-queries, and so does every
// `notes/page_changed`, coalesced; a newer query always wins over an older
// one. A committed cell shows at once and holds until a query that started
// after its write lands, or goes back when the write is refused.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../../api";
import type { DatabaseGroup, DatabaseRow, NoteDatabase, NoteDatabaseView, NoteProperty, NoteSchemaChange, NoteViewInput, QueryDatabaseParams, Workspace } from "../../../types";
import { DatabaseBoard, GROUP_LIMIT } from "./DatabaseBoard";
import { DatabaseCalendar } from "./DatabaseCalendar";
import { DatabaseList } from "./DatabaseList";
import { DatabaseTable, type TableActions } from "./DatabaseTable";
import type { CellHost } from "./PropertyCell";
import { columnsOf, viewInputOf, withColumn, type Column } from "./formatValue";
import { addOption, saveOptions } from "./schemaWrites";
import { useLatestOnly } from "./useLatestOnly";
import { useViewOverride } from "./useViewOverride";
import type { ViewBodyProps } from "./viewBody";
import { groupableColumns, filterColumns, type FieldColumn } from "./viewModel";
import { ViewTabs, type ViewActions } from "./ViewTabs";
import { ViewToolbar } from "./ViewToolbar";

export const FIRST_PAGE = 100;
export const LATER_PAGE = 500;
const COALESCE_MS = 30;

/** How many rows each group of a grouped list carries. */
const LIST_GROUP_LIMIT = 200;

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
  const [groups, setGroups] = useState<DatabaseGroup[] | null>(null);
  /** Bumps on every query, so a body with its own reads re-reads. */
  const [version, setVersion] = useState(0);
  const override = useViewOverride();
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
  // Per view, so queued edits to different views (a reorder moves two) each
  // build on their own last edit.
  const viewDrafts = useRef(new Map<string, NoteDatabaseView>());
  const withDrafts = (database: NoteDatabase) => [...viewDrafts.current.values()].reduce(withView, database);
  useEffect(() => { if (given) setDatabase(withDrafts(given)); }, [given]); // eslint-disable-line react-hooks/exhaustive-deps

  // Schema reads and writes land newest first, whatever order they answer in.
  const schemaOrder = useLatestOnly();
  const readDatabase = useCallback(async () => {
    const ticket = schemaOrder.take();
    try {
      const page = await bridgeApi.readNotePage(databaseId);
      if (page.database && schemaOrder.accept(ticket)) setDatabase(withDrafts(page.database));
    } catch (error) { fail(error); }
  }, [databaseId, fail, schemaOrder]);
  useEffect(() => { if (!given) void readDatabase(); }, [given, readDatabase]);

  const views = useMemo(() => [...(database?.views ?? [])].sort((a, b) => a.order - b.order), [database]);
  const view: NoteDatabaseView | undefined = views.find(candidate => candidate.id === activeId) ?? views[0];
  const queryViewId = view?.id ?? null;
  // The saved view by id, or an override's fields spelled out (without the
  // view id, so a grouping it drops is really dropped).
  const effective = view ? override.effective(view) : null;
  const dirty = view ? override.dirty(view) : false;
  const groupLimit = view?.type === "list" ? LIST_GROUP_LIMIT : GROUP_LIMIT;
  const queryKey = JSON.stringify([queryViewId, dirty ? effective : null, groupLimit]);
  const viewParams = useMemo((): Omit<QueryDatabaseParams, "databaseId" | "limit" | "cursor"> => {
    const [viewId, spelled, limit] = JSON.parse(queryKey) as [string | null, { filter: unknown; sorts: QueryDatabaseParams["sorts"]; groupBy: QueryDatabaseParams["groupBy"] } | null, number];
    return spelled ? { filter: spelled.filter, sorts: spelled.sorts, groupBy: spelled.groupBy, groupLimit: limit } : { viewId, groupLimit: limit };
  }, [queryKey]);

  /** The rows, the first page at once and the rest by cursor behind it. */
  const query = useCallback(async () => {
    const mine = ++ticket.current;
    if (!painted.current) performance.mark?.("notes.table.query-start");
    try {
      // The first load paints its first page at once. A refresh replaces
      // nothing until it holds every page, so the table never shrinks under
      // a scrolled view or an open editor.
      const refresh = loaded.current;
      let result = await bridgeApi.queryDatabase({ databaseId, ...viewParams, limit: FIRST_PAGE });
      if (mine !== ticket.current) return;
      let all = result.rows;
      const groupsNow = result.groups ?? null;
      const land = () => {
        setRows(all);
        setGroups(groupsNow);
        setVersion(current => current + 1);
        setPending(current => current.filter(item => item.after >= mine));
        loaded.current = true;
      };
      if (!refresh) land();
      while (result.nextCursor) {
        result = await bridgeApi.queryDatabase({ databaseId, ...viewParams, limit: LATER_PAGE, cursor: result.nextCursor });
        if (mine !== ticket.current) return;
        all = [...all, ...result.rows];
        if (!refresh) setRows(all);
      }
      if (refresh) land();
    } catch (error) {
      if (mine === ticket.current) fail(error);
    }
  }, [databaseId, viewParams, fail]);

  useEffect(() => { loaded.current = false; if (database) void query(); }, [database ? database.id : null, queryKey, query]); // eslint-disable-line react-hooks/exhaustive-deps

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
      else if (item.key === "kind" || item.key === "state") note[item.key] = String(item.value);
      else if (item.value === null) delete values[item.key];
      else values[item.key] = item.value;
    }
    return { note, values };
  }), [rows, pending]);

  const commit = useCallback(async (row: DatabaseRow, column: Pick<Column, "id">, value: unknown): Promise<boolean> => {
    const entry: Pending = { row: row.note.id, key: column.id, value, after: Number.POSITIVE_INFINITY };
    setPending(current => [...current.filter(item => !(item.row === entry.row && item.key === entry.key)), entry]);
    try {
      if (column.id === "title") await bridgeApi.updateNote({ id: row.note.id, title: String(value ?? "") });
      else if (column.id === "project") await bridgeApi.updateNote({ id: row.note.id, ...(value ? { workspaceId: String(value) } : { clearWorkspace: true }) });
      else if (column.id === "kind" || column.id === "state") await bridgeApi.updateNote({ id: row.note.id, [column.id]: String(value) });
      else await bridgeApi.updateNote({ id: row.note.id, values: { [column.id]: value ?? null } });
      // Held until a query that starts from here lands.
      entry.after = ticket.current;
      setPending(current => [...current]);
      void query();
      return true;
    } catch (error) {
      setPending(current => current.filter(item => item !== entry));
      fail(error);
      return false;
    }
  }, [query, fail]);

  const change = useCallback(async (changes: NoteSchemaChange[]): Promise<NoteDatabase | null> => {
    const ticket = schemaOrder.take();
    try {
      const next = await bridgeApi.updateDatabaseSchema({ id: databaseId, changes });
      if (schemaOrder.accept(ticket)) setDatabase(next);
      void query();
      return next;
    } catch (error) { fail(error); return null; }
  }, [databaseId, query, fail, schemaOrder]);

  // View edits apply at once, each built on the one before it (still unsaved
  // or not), and save in order; only the newest answer replaces the view.
  const latestDatabase = useRef(database);
  latestDatabase.current = database;
  const latestView = useRef(view);
  latestView.current = view;
  const saves = useRef<Promise<unknown>>(Promise.resolve());
  const saveSeq = useRef(0);
  /** Queue a view edit; resolves to whether it was stored. */
  const saveView = useCallback((patch: (base: NoteDatabaseView, columns: Column[]) => Partial<NoteViewInput>, target?: NoteDatabaseView): Promise<boolean> => {
    const current = latestDatabase.current;
    const shown = target ?? latestView.current;
    if (!current || !shown) return Promise.resolve(false);
    const base = viewDrafts.current.get(shown.id) ?? shown;
    const input: NoteViewInput = { ...viewInputOf(base), ...patch(base, columnsOf(current, base)) };
    const optimistic: NoteDatabaseView = {
      id: base.id, name: input.name, type: input.type, order: input.order ?? base.order, filter: input.filter,
      sorts: input.sorts ?? [], groupBy: input.groupBy ?? null, dateProperty: input.dateProperty ?? null, properties: input.properties ?? [],
    };
    viewDrafts.current.set(optimistic.id, optimistic);
    setDatabase(held => held && withView(held, optimistic));
    const mine = ++saveSeq.current;
    const write = saves.current.then(() => bridgeApi.saveDatabaseView({ databaseId, view: input }));
    saves.current = write.then(saved => {
      if (mine !== saveSeq.current) return;
      // The newest save landed: everything queued before it has too.
      viewDrafts.current.clear();
      setDatabase(held => held && withView(held, saved));
      void query();
    }, error => {
      if (mine === saveSeq.current) { viewDrafts.current.clear(); void readDatabase(); }
      fail(error);
    });
    return write.then(() => true, () => false);
  }, [databaseId, query, readDatabase, fail]);

  const host: CellHost = useMemo(() => ({
    addOption: async (property: NoteProperty, option) => {
      const ticket = schemaOrder.take();
      try {
        const next = await addOption(databaseId, property.id, option);
        if (schemaOrder.accept(ticket)) setDatabase(next);
        void query();
        return next.properties.find(candidate => candidate.id === property.id) ?? null;
      } catch (error) { fail(error); return null; }
    },
    searchRows: async (target, text) => (await bridgeApi.queryDatabase({ databaseId: target, query: text.trim() || null, limit: 20 })).rows.map(row => ({ id: row.note.id, title: row.note.title ?? "" })),
    rowTitle: id => related.get(id) ?? rows?.find(row => row.note.id === id)?.note.title ?? undefined,
    workspaces,
    chatTitle,
    onOpenChat,
  }), [databaseId, query, fail, schemaOrder, related, rows, workspaces, chatTitle, onOpenChat]);

  const actions: TableActions = useMemo(() => ({
    rename: (property, name) => void change([{ op: "rename", propertyId: property.id, name }]),
    retype: (property, type) => void change([{ op: "retype", propertyId: property.id, type }]),
    setOptions: (property, options, seen) => {
      const ticket = schemaOrder.take();
      void saveOptions(databaseId, property.id, options, seen).then(next => { if (schemaOrder.accept(ticket)) setDatabase(next); void query(); }, fail);
    },
    remove: property => void change([{ op: "delete", propertyId: property.id }]),
    addProperty: (name, type) => void change([{ op: "add", property: { name, type, ...(type === "status" ? { options: [{ name: "To do", group: "todo" }, { name: "In progress", group: "inProgress" }, { name: "Done", group: "done" }] } : {}) } }]),
    sort: (column, direction) => saveView(() => ({ sorts: [{ property: column.id, direction }] })),
    hide: column => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { visible: false }) })),
    show: column => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { visible: true }) })),
    resize: (column, width) => saveView((_, columns) => ({ properties: withColumn(columns, column.id, { width }) })),
  }), [databaseId, change, saveView, query, fail, schemaOrder]);

  const newRow = useCallback(async (initial?: { column: FieldColumn; value: unknown }) => {
    try {
      const note = await bridgeApi.createNotePage({ title: null, content: "", parentId: databaseId, ...(initial?.column.id === "kind" ? { kind: String(initial.value) } : {}) });
      if (initial && initial.value !== null && initial.value !== undefined && initial.column.id !== "kind") await commit({ note, values: {} }, initial.column, initial.value);
      void query();
      return note.id;
    } catch (error) { fail(error); return null; }
  }, [databaseId, query, fail, commit]);

  // Views: made, renamed, retyped, duplicated, moved and deleted.
  const views_ = useRef(views);
  views_.current = views;
  // Orders go through the same queue as every other view edit, each built on
  // that view's latest, so a slow reorder never writes an older field back.
  const saveOrder = useCallback(async (ordered: NoteDatabaseView[]) => {
    const moved = ordered.map((candidate, order) => ({ candidate, order })).filter(({ candidate, order }) => candidate.order !== order);
    await Promise.all(moved.map(({ candidate, order }) => saveView(() => ({ order }), candidate)));
  }, [saveView]);
  const typeDefaults = useCallback((type: string, current?: NoteDatabaseView): Partial<NoteViewInput> => {
    if (!database) return {};
    if (type === "board" && !current?.groupBy) return { groupBy: groupableColumns(database).find(column => column.type !== "date") ? { property: groupableColumns(database).find(column => column.type !== "date")!.id } : null };
    if (type === "calendar" && !current?.dateProperty) return { dateProperty: filterColumns(database).find(column => column.type === "date")?.id ?? "created" };
    return {};
  }, [database]);
  const viewActions: ViewActions = useMemo(() => ({
    create: (type, name) => void (async () => {
      try {
        const order = Math.max(-1, ...views_.current.map(candidate => candidate.order)) + 1;
        const saved = await bridgeApi.saveDatabaseView({ databaseId, view: { name, type, order, ...typeDefaults(type) } });
        setDatabase(current => current && { ...current, views: [...current.views, saved] });
        setActiveId(saved.id);
      } catch (error) { fail(error); }
    })(),
    rename: (target, name) => saveView(() => ({ name }), target),
    changeType: (target, type) => saveView(() => ({ type, ...typeDefaults(type, target) }), target),
    duplicate: target => void (async () => {
      try {
        const copy = await bridgeApi.saveDatabaseView({ databaseId, view: { ...viewInputOf(target), id: null, name: `${target.name} copy`, order: target.order + 1 } });
        const ordered = [...views_.current];
        ordered.splice(ordered.findIndex(candidate => candidate.id === target.id) + 1, 0, copy);
        setDatabase(current => current && { ...current, views: [...current.views, copy] });
        await saveOrder(ordered);
        setActiveId(copy.id);
      } catch (error) { fail(error); }
    })(),
    remove: target => void (async () => {
      if (views_.current.length <= 1) return;
      try {
        await bridgeApi.deleteDatabaseView({ databaseId, viewId: target.id });
        setDatabase(current => current && { ...current, views: current.views.filter(candidate => candidate.id !== target.id) });
        setActiveId(current => current === target.id ? null : current);
      } catch (error) { fail(error); }
    })(),
    move: (target, index) => {
      const ordered = views_.current.filter(candidate => candidate.id !== target.id);
      ordered.splice(Math.max(0, Math.min(index, ordered.length)), 0, target);
      void saveOrder(ordered).catch(fail);
    },
  }), [databaseId, typeDefaults, saveView, saveOrder, fail]);

  const run = useCallback((params: Omit<QueryDatabaseParams, "databaseId">) => bridgeApi.queryDatabase({ databaseId, ...params }), [databaseId]);

  if (!database) return <SkeletonRows label="Loading the database" rows={4} meta={false} className="my-3" />;

  const body: ViewBodyProps | null = view && effective ? {
    database, view, query: effective, rows: shown, groups, run, version, host,
    onCommit: commit, onNewRow: newRow, onOpenRow: onOpenPage,
  } : null;
  return <section aria-label="Database" className="min-w-0">
    <ViewTabs views={views} activeId={view?.id ?? null} onSelect={setActiveId} actions={viewActions} />
    {view && effective && <ViewToolbar
      database={database}
      view={view}
      query={effective}
      dirty={dirty}
      workspaces={workspaces}
      onChange={patch => override.change(view, patch)}
      onReset={() => override.reset(view)}
      onSave={() => void override.save(view, query => saveView(() => ({ filter: query.filter, sorts: query.sorts, groupBy: query.groupBy }), view))}
    />}
    {rows === null || !body
      ? <SkeletonRows label="Loading rows" rows={4} meta={false} framed={false} className="py-1" />
      : view?.type === "board" ? <DatabaseBoard {...body} />
        : view?.type === "calendar" ? <DatabaseCalendar {...body} />
          : view?.type === "list" ? <DatabaseList {...body} />
            : <DatabaseTable database={database} view={view} rows={shown} host={host} actions={actions} onCommit={(row, column, value) => void commit(row, column, value)} onNewRow={() => newRow()} onOpenRow={onOpenPage} bounded={bounded} />}
  </section>;
}
