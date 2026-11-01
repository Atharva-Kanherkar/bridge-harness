// Your words, as the editor holds them. The draft always knows the version it
// came from (`base`): a save sends the full text with that seq and the core
// merges it with anything that landed since, so an agent writing to the open
// page never loses the person's typing, and the person never overwrites the
// agent. One save is in flight at a time.

import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "../../api";
import { rebaseDraft } from "../../notesDraft";
import type { NotePage } from "../../types";

export const AUTOSAVE_MS = 800;

export type DraftStatus = "saved" | "unsaved" | "saving";

export interface NoteDraft {
  text: string;
  status: DraftStatus;
  /** The page changed under unsaved typing; the typing merges on save. */
  changedUnderneath: boolean;
  setText: (text: string) => void;
  /** Save now, and resolve once everything typed so far is stored: the save in
   *  flight, then whatever was typed during it. False when a save failed. */
  flush: () => Promise<boolean>;
  /** A fresh read of the page: adopted unless the person has unsaved typing. */
  load: (page: NotePage) => void;
}

export function useNoteDraft(initial: NotePage, onSaved: (page: NotePage) => void, onError: (message: string) => void): NoteDraft {
  const id = initial.note.id;
  const base = useRef({ text: initial.markdown, seq: initial.seq });
  const draft = useRef(initial.markdown);
  const saving = useRef(false);
  const inflight = useRef<Promise<boolean> | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const [text, setTextState] = useState(initial.markdown);
  const [status, setStatus] = useState<DraftStatus>("saved");
  const [changedUnderneath, setChangedUnderneath] = useState(false);
  const callbacks = useRef({ onSaved, onError });
  callbacks.current = { onSaved, onError };

  const show = useCallback((next: string) => { draft.current = next; setTextState(next); }, []);

  const saveOnce = useCallback(async (): Promise<boolean> => {
    saving.current = true;
    setStatus("saving");
    const sent = draft.current;
    try {
      await bridgeApi.editNotePage({ id, text: sent, baseSeq: base.current.seq });
      const page = await bridgeApi.readNotePage(id);
      // Typing that happened during the round trip came from `sent`, which the
      // core never stored as a version; replay it onto the merged text.
      const next = draft.current === sent ? page.markdown : rebaseDraft(sent, draft.current, page.markdown).text;
      base.current = { text: page.markdown, seq: page.seq };
      show(next);
      setChangedUnderneath(false);
      callbacks.current.onSaved(page);
      return true;
    } catch (error) {
      callbacks.current.onError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      saving.current = false;
      const pending = draft.current !== base.current.text;
      setStatus(pending ? "unsaved" : "saved");
      if (pending) timer.current = setTimeout(() => void saveRef.current(), AUTOSAVE_MS);
    }
  }, [id, show]);

  const save = useCallback((): Promise<boolean> => {
    clearTimeout(timer.current);
    if (inflight.current) return inflight.current;
    if (draft.current === base.current.text) return Promise.resolve(true);
    const run = saveOnce().finally(() => { inflight.current = null; });
    inflight.current = run;
    return run;
  }, [saveOnce]);
  const saveRef = useRef(save);
  saveRef.current = save;

  const flush = useCallback(async (): Promise<boolean> => {
    // A save can end with newer typing still pending; keep going until the
    // stored text is the text on screen.
    while (inflight.current || draft.current !== base.current.text) {
      if (!(await save())) return false;
    }
    return true;
  }, [save]);

  const setText = useCallback((next: string) => {
    draft.current = next;
    setTextState(next);
    if (saving.current) return;
    setStatus(next === base.current.text ? "saved" : "unsaved");
    clearTimeout(timer.current);
    timer.current = setTimeout(() => void save(), AUTOSAVE_MS);
  }, [save]);

  const load = useCallback((page: NotePage) => {
    // Only newer versions: a late read of an older one must not roll back.
    if (page.note.id !== id || saving.current || page.seq <= base.current.seq) return;
    if (draft.current === base.current.text) {
      base.current = { text: page.markdown, seq: page.seq };
      show(page.markdown);
      setStatus("saved");
      return;
    }
    if (page.markdown !== base.current.text) setChangedUnderneath(true);
  }, [id, show]);

  // Leaving the page (or the screen) saves what was typed.
  useEffect(() => () => {
    clearTimeout(timer.current);
    if (!saving.current && draft.current !== base.current.text) {
      void bridgeApi.editNotePage({ id, text: draft.current, baseSeq: base.current.seq }).catch(error => callbacks.current.onError(error instanceof Error ? error.message : String(error)));
    }
  }, [id]);

  return { text, status, changedUnderneath, setText, flush, load };
}
