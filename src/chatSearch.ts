// Cross-chat search from the sidebar: "find that chat" from a vague memory.
//
// The index answers as you type. Enter either opens the index's clear winner
// or, when the index is unsure, asks for the deep stage, which may run a
// small model turn. The deep result replaces the typed one; a response that
// arrives after the query moved on is dropped, so a slow deep search can
// never overwrite the cards for a newer query.

import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import type { SearchChatsResult } from "./protocol/generated/protocol";

export const CHAT_SEARCH_MIN_CHARS = 2;
export const CHAT_SEARCH_DEBOUNCE_MS = 180;

const FIND_COMMAND = /^\/find\b\s*([\s\S]*)$/i;

/** The query of a `/find <query>` composer turn, or null when the text is not
 *  one. An empty query is still a find request: it opens an empty search. */
export function parseFindCommand(text: string): string | null {
  const match = FIND_COMMAND.exec(text.trim());
  return match ? match[1].trim() : null;
}

/** What Enter in the search field should do with the current result. */
export function enterAction(result: SearchChatsResult | undefined, deepRunning: boolean): "open" | "deep" | "none" {
  if (!result || deepRunning) return "none";
  // Once the deep stage has answered, or cannot run, the top card is the answer.
  if (result.stage !== "index" || result.confident || !result.deepAvailable) {
    return result.hits.length ? "open" : "none";
  }
  return "deep";
}

export interface ChatSearchState {
  result?: SearchChatsResult;
  /** The index request for the current query is in flight. */
  loading: boolean;
  /** The deep request for the current query is in flight. */
  deepRunning: boolean;
  error?: string;
}

/** Index results for `query` as it is typed, plus an explicit deep run. */
export function useChatSearch(query: string): ChatSearchState & { runDeep: () => void } {
  const [state, setState] = useState<ChatSearchState>({ loading: false, deepRunning: false });
  // Every request carries the generation it was made in; only the latest
  // generation may write state.
  const generation = useRef(0);
  const trimmed = query.trim();

  useEffect(() => {
    const current = ++generation.current;
    if (trimmed.length < CHAT_SEARCH_MIN_CHARS) {
      setState({ loading: false, deepRunning: false });
      return;
    }
    setState(previous => ({ ...previous, loading: true, deepRunning: false, error: undefined }));
    const timer = window.setTimeout(() => {
      bridgeApi.searchChats(trimmed)
        .then(result => { if (generation.current === current) setState({ result, loading: false, deepRunning: false }); })
        .catch(error => {
          if (generation.current === current) setState({ loading: false, deepRunning: false, error: error instanceof Error ? error.message : String(error) });
        });
    }, CHAT_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [trimmed]);

  const runDeep = useCallback(() => {
    if (trimmed.length < CHAT_SEARCH_MIN_CHARS) return;
    const current = ++generation.current;
    setState(previous => ({ ...previous, loading: false, deepRunning: true, error: undefined }));
    bridgeApi.searchChats(trimmed, { deep: true })
      .then(result => { if (generation.current === current) setState({ result, loading: false, deepRunning: false }); })
      .catch(error => {
        if (generation.current === current) {
          setState(previous => ({ ...previous, deepRunning: false, error: error instanceof Error ? error.message : String(error) }));
        }
      });
  }, [trimmed]);

  return { ...state, runDeep };
}
