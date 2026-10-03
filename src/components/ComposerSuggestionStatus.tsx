export function ComposerSuggestionStatus({ error, onRetry }: { error?: string; onRetry: () => void }) {
  if (!error) return null;
  return <p role="status" className="mt-2 text-xs text-muted-foreground" title={error}>
    Inline suggestions unavailable. Check Settings → Composer.{" "}
    <button type="button" onClick={onRetry} className="rounded text-foreground underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-ring">Retry</button>
  </p>;
}
