// What every view body (board, calendar, list) gets from `DatabaseView`.

import type { DatabaseGroup, DatabaseQueryResult, DatabaseRow, NoteDatabase, NoteDatabaseView, QueryDatabaseParams } from "../../../types";
import type { CellHost } from "./PropertyCell";
import type { ViewQuery } from "./useViewOverride";
import { countConditions, isGroup, MAX_CONDITIONS, readFilter, writeFilter, type Condition, type FieldColumn, type Group } from "./viewModel";

export interface ViewBodyProps {
  database: NoteDatabase;
  view: NoteDatabaseView;
  /** What the view shows now: the unsaved override, else the saved view. */
  query: ViewQuery;
  rows: DatabaseRow[];
  groups: DatabaseGroup[] | null;
  /** A query of this database, for a body's own reads (a column's next page, a month). */
  run: (params: Omit<QueryDatabaseParams, "databaseId">) => Promise<DatabaseQueryResult>;
  /** Bumps whenever the view's data may have changed, so a body re-reads its own. */
  version: number;
  host: CellHost;
  /** Write one value; resolves to whether it was stored (a refusal is already reported). */
  onCommit: (row: DatabaseRow, column: FieldColumn, value: unknown) => Promise<boolean>;
  /** Make a row, with one value set if given; resolves to its id. */
  onNewRow: (initial?: { column: FieldColumn; value: unknown }) => Promise<string | null>;
  onOpenRow: (id: string) => void;
}

/** `base` with `conditions` added, kept within the planner's limits (groups
 *  two levels deep, 20 conditions): a top `and` takes them as items, a top
 *  `or` gets them in each branch. Null when no form fits; the caller then
 *  reads `base` alone and applies the conditions to the rows itself. */
export function composeFilter(base: unknown, conditions: Condition[]): unknown | null {
  const top = readFilter(base);
  // An empty group is true, so a top `or` holding one matches every row: the
  // base adds nothing then, and distributing would drop that true branch.
  const alwaysTrue = top.join === "or" && top.items.some(item => isGroup(item) && !item.items.length);
  let composed: Group;
  if (!top.items.length || alwaysTrue) composed = { join: "and", items: conditions };
  else if (top.join === "and") composed = { join: "and", items: [...top.items, ...conditions] };
  else composed = {
    join: "or",
    items: top.items.flatMap((item): (Condition | Group)[] => {
      if (!isGroup(item)) return [{ join: "and", items: [item, ...conditions] }];
      // A group in an `or` holds only conditions (two levels at most).
      return item.join === "and" ? [{ join: "and", items: [...item.items, ...conditions] }] : item.items.map(sub => ({ join: "and" as const, items: [sub, ...conditions] }));
    }),
  };
  return countConditions(composed) <= MAX_CONDITIONS ? writeFilter(composed) : null;
}

/** Every row of a query, following its cursors. */
export async function readAll(run: ViewBodyProps["run"], params: Omit<QueryDatabaseParams, "databaseId" | "limit" | "cursor">): Promise<DatabaseRow[]> {
  const rows: DatabaseRow[] = [];
  let cursor: string | null = null;
  do {
    const result: DatabaseQueryResult = await run({ ...params, limit: 500, cursor });
    rows.push(...result.rows);
    cursor = result.nextCursor ?? null;
  } while (cursor);
  return rows;
}

/** The rows of `base` that also meet `conditions`: by query when the two
 *  compose within the planner's limits, else by `matches` over every row. */
export async function readWith(run: ViewBodyProps["run"], base: unknown, sorts: QueryDatabaseParams["sorts"], conditions: Condition[], matches: (row: DatabaseRow) => boolean): Promise<DatabaseRow[]> {
  const composed = composeFilter(base, conditions);
  return composed !== null ? readAll(run, { filter: composed, sorts }) : (await readAll(run, { filter: base ?? null, sorts })).filter(matches);
}
