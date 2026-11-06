// What every view body (board, calendar, list) gets from `DatabaseView`.

import type { DatabaseGroup, DatabaseQueryResult, DatabaseRow, NoteDatabase, NoteDatabaseView, QueryDatabaseParams } from "../../../types";
import type { CellHost } from "./PropertyCell";
import type { ViewQuery } from "./useViewOverride";
import type { FieldColumn } from "./viewModel";

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

/** A filter with `condition` added to `base` (which may be none). */
export function withCondition(base: unknown, ...conditions: unknown[]): unknown {
  return base ? { and: [base, ...conditions] } : { and: conditions };
}
