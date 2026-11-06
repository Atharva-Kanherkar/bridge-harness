// A view's filter, sorts and grouping as the toolbar edits them: an unsaved
// override, per view, that applies at once and is saved for everyone or
// dropped. It lives in component state only, so a quick look never changes
// the saved view.

import { useCallback, useMemo, useState } from "react";
import type { NoteDatabaseView, NoteGroupBy, NoteSort } from "../../../types";
import { readFilter, writeFilter } from "./viewModel";

export interface ViewQuery {
  filter: unknown;
  sorts: NoteSort[];
  groupBy: NoteGroupBy | null;
}

/** The saved view's query, normalized. */
export function savedQuery(view: NoteDatabaseView): ViewQuery {
  return { filter: writeFilter(readFilter(view.filter)), sorts: view.sorts ?? [], groupBy: view.groupBy ? { property: view.groupBy.property, dateBucket: view.groupBy.dateBucket ?? null } : null };
}

const normal = (query: ViewQuery) => JSON.stringify([writeFilter(readFilter(query.filter)), query.sorts.map(sort => [sort.property, sort.direction]), query.groupBy ? [query.groupBy.property, query.groupBy.dateBucket ?? null] : null]);
export const sameQuery = (a: ViewQuery, b: ViewQuery) => normal(a) === normal(b);

export interface ViewOverride {
  /** What the view shows now: the override, else the saved view. */
  effective: (view: NoteDatabaseView) => ViewQuery;
  /** Whether the view shows something other than what is saved. */
  dirty: (view: NoteDatabaseView) => boolean;
  change: (view: NoteDatabaseView, patch: Partial<ViewQuery>) => void;
  reset: (view: NoteDatabaseView) => void;
  /** Save the override for everyone through `save`, then drop it. */
  save: (view: NoteDatabaseView, save: (query: ViewQuery) => Promise<void> | void) => Promise<void>;
}

export function useViewOverride(): ViewOverride {
  const [overrides, setOverrides] = useState<Record<string, ViewQuery>>({});
  const effective = useCallback((view: NoteDatabaseView) => overrides[view.id] ?? savedQuery(view), [overrides]);
  const dirty = useCallback((view: NoteDatabaseView) => !!overrides[view.id] && !sameQuery(overrides[view.id], savedQuery(view)), [overrides]);
  const drop = useCallback((view: NoteDatabaseView) => setOverrides(current => {
    if (!(view.id in current)) return current;
    const next = { ...current };
    delete next[view.id];
    return next;
  }), []);
  const change = useCallback((view: NoteDatabaseView, patch: Partial<ViewQuery>) => setOverrides(current => {
    const next = { ...(current[view.id] ?? savedQuery(view)), ...patch };
    // Edited back to what is saved: no override.
    if (sameQuery(next, savedQuery(view))) {
      const rest = { ...current };
      delete rest[view.id];
      return rest;
    }
    return { ...current, [view.id]: next };
  }), []);
  const save = useCallback(async (view: NoteDatabaseView, write: (query: ViewQuery) => Promise<void> | void) => {
    const query = overrides[view.id];
    if (!query) return;
    await write({ ...query, filter: writeFilter(readFilter(query.filter)) });
    drop(view);
  }, [overrides, drop]);
  return useMemo(() => ({ effective, dirty, change, reset: drop, save }), [effective, dirty, change, drop, save]);
}
