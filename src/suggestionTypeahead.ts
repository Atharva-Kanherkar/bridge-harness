import type { SuggestCompletionResult } from "./protocol/generated/protocol";

// The composer typeahead's debounce and cancellation, pulled out of App.tsx so
// it is a plain function testable with fake timers rather than something only
// exercisable by mounting the whole app.
//
// "Latest wins" is enforced with a generation counter (the same discipline
// `readWorkBoard` uses for its own stale-response guard): each call bumps a
// shared counter, and a response is applied only if the counter still reads
// the value this call bumped it to. A request whose draft was superseded
// before it resolved is silently dropped rather than racing a newer one onto
// the screen.

const DEBOUNCE_MS = 400;

export function scheduleSuggestion(options: {
  text: string;
  enabled: boolean;
  request: (text: string) => Promise<SuggestCompletionResult>;
  onResult: (result: SuggestCompletionResult | undefined) => void;
  generation: { current: number };
  debounceMs?: number;
}): () => void {
  const { text, enabled, request, onResult, generation } = options;
  // Clearing eagerly means a request that is superseded before it even fires
  // (the debounce window resets on every keystroke) leaves no stale ghost text
  // showing for a draft the user has already moved past.
  onResult(undefined);
  if (!enabled || !text.trim()) {
    return () => {};
  }
  const mine = ++generation.current;
  const timer = window.setTimeout(() => {
    request(text)
      .then(result => {
        if (generation.current !== mine) return;
        onResult(result.suggestion ? result : undefined);
      })
      .catch(() => {
        if (generation.current === mine) onResult(undefined);
      });
  }, options.debounceMs ?? DEBOUNCE_MS);
  return () => window.clearTimeout(timer);
}
