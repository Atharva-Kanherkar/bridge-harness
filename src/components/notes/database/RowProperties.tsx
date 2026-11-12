// A row page's properties: every property of its database, in schema order,
// each with the same editor its table cell uses, above Your words.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { bridgeApi } from "../../../api";
import type { Note, NoteDatabase, NoteProperty, NotePage, NoteSchemaChange, Workspace } from "../../../types";
import { Floating } from "./Floating";
import { PropertyCell, type CellHost } from "./PropertyCell";
import { AddProperty } from "./SchemaMenu";
import { addOption } from "./schemaWrites";
import { useLatestOnly } from "./useLatestOnly";
import { typeIcon } from "./DatabaseTable";
import { TYPE_LABELS } from "./formatValue";

export interface RowPropertiesProps {
  page: NotePage;
  workspaces: Workspace[];
  /** Every page in the space, to name related rows. */
  notes?: Note[];
  chatTitle?: (sessionId: string) => string | undefined;
  onOpenChat?: (sessionId: string) => void;
  onChanged: () => void;
  onError: (message: string) => void;
}

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export function RowProperties({ page, workspaces, notes, chatTitle, onOpenChat, onChanged, onError }: RowPropertiesProps) {
  const databaseId = page.databaseId!;
  const [database, setDatabase] = useState<NoteDatabase | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>(page.values ?? {});
  const [adding, setAdding] = useState(false);
  const addAnchor = useRef<HTMLDivElement>(null);
  const fail = useCallback((error: unknown) => onError(message(error)), [onError]);
  useEffect(() => setValues(page.values ?? {}), [page.values]);
  // Schema reads and writes land newest first, whatever order they answer in.
  const schemaOrder = useLatestOnly();
  // The schema moves without this row's seq: re-read it whenever the database changes.
  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    let burst: ReturnType<typeof setTimeout> | undefined;
    const read = () => {
      const ticket = schemaOrder.take();
      void bridgeApi.readNotePage(databaseId).then(next => { if (alive && schemaOrder.accept(ticket)) setDatabase(next.database ?? null); }, fail);
    };
    read();
    void bridgeApi.onNotesPageChanged(payload => {
      if (payload.pageId !== databaseId) return;
      clearTimeout(burst);
      burst = setTimeout(read, 30);
    }).then(unlisten => { if (alive) stop = unlisten; else unlisten(); });
    return () => { alive = false; clearTimeout(burst); stop?.(); };
  }, [databaseId, page.seq, fail, schemaOrder]);

  const change = useCallback(async (changes: NoteSchemaChange[]) => {
    const ticket = schemaOrder.take();
    try {
      const next = await bridgeApi.updateDatabaseSchema({ id: databaseId, changes });
      if (schemaOrder.accept(ticket)) setDatabase(next);
      return next;
    } catch (error) { fail(error); return null; }
  }, [databaseId, fail, schemaOrder]);

  const host: CellHost = useMemo(() => ({
    addOption: async (property: NoteProperty, option) => {
      const ticket = schemaOrder.take();
      try {
        const next = await addOption(databaseId, property.id, option);
        if (schemaOrder.accept(ticket)) setDatabase(next);
        return next.properties.find(candidate => candidate.id === property.id) ?? null;
      } catch (error) { fail(error); return null; }
    },
    searchRows: async (target, text) => (await bridgeApi.queryDatabase({ databaseId: target, query: text.trim() || null, limit: 20 })).rows.map(row => ({ id: row.note.id, title: row.note.title ?? "" })),
    rowTitle: id => notes?.find(note => note.id === id)?.title ?? undefined,
    workspaces,
    chatTitle,
    onOpenChat,
  }), [databaseId, fail, schemaOrder, notes, workspaces, chatTitle, onOpenChat]);

  const commit = async (property: NoteProperty, value: unknown) => {
    const before = values;
    setValues(current => { const next = { ...current }; if (value === null) delete next[property.id]; else next[property.id] = value; return next; });
    try {
      await bridgeApi.updateNote({ id: page.note.id, values: { [property.id]: value ?? null } });
      onChanged();
    } catch (error) {
      setValues(before);
      fail(error);
    }
  };

  if (!database) return null;
  const properties = [...database.properties].sort((a, b) => a.order - b.order);
  // Lines of the row's file the importer could not set, for properties the
  // database still has. Each goes once the cell is set or the file is fixed.
  const notices = (page.notices ?? []).flatMap(notice => {
    const property = properties.find(candidate => candidate.id === notice.propertyId);
    return property ? [{ notice, property }] : [];
  });
  return <div aria-label="Row properties">
    {notices.map(({ notice, property }) => <p key={`${notice.kind}-${notice.propertyId}`} role="status" className="m-0 mb-1 flex flex-wrap items-baseline gap-x-1 rounded-md bg-muted px-2 py-1.5 text-[12.5px] text-foreground">
      {property.name} in the file could not be read as a {(TYPE_LABELS[notice.expected] ?? notice.expected).toLowerCase()}:{" "}
      <code className="rounded bg-background px-1 font-mono text-[12px] text-muted-foreground">{notice.raw}</code>
    </p>)}
    {properties.map(property => <div key={property.id} className="flex min-h-8 items-start gap-2">
      <dt className="flex w-28 shrink-0 items-center gap-1.5 py-1.5 text-[13px] text-muted-foreground">
        <span className="shrink-0">{typeIcon(property.type)}</span>
        <span className="min-w-0 truncate">{property.name}</span>
      </dt>
      <dd className="m-0 min-w-0 flex-1">
        <PropertyCell column={{ id: property.id, name: property.name, type: property.type, property }} value={values[property.id]} host={host} onCommit={value => void commit(property, value)} />
      </dd>
    </div>)}
    <div ref={addAnchor} className="relative">
      <button type="button" onClick={() => setAdding(current => !current)} aria-expanded={adding} className="-ml-1.5 mt-0.5 inline-flex h-8 items-center gap-1.5 rounded-md px-1.5 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
        <Plus size={14} aria-hidden="true" />Add a property
      </button>
      {adding && <Floating anchor={addAnchor} label="Add a property" onClose={() => setAdding(false)}>
        <AddProperty hidden={[]} onAdd={(name, type) => { setAdding(false); void change([{ op: "add", property: { name, type, ...(type === "status" ? { options: [{ name: "To do", group: "todo" }, { name: "In progress", group: "inProgress" }, { name: "Done", group: "done" }] } : {}) } }]); }} />
      </Floating>}
    </div>
  </div>;
}
