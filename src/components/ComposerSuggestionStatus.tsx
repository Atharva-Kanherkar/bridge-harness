import { Button } from "@/components/ui/button";
export function ComposerSuggestionStatus({ error, onRetry }: { error?: string; onRetry: () => void }) {
  if (!error) return null;
  return <p role="status" className="mt-2 text-xs text-muted-foreground" title={error}>
    Inline suggestions unavailable. Check Settings → Composer.{" "}
    <Button variant="link" size="inline" onClick={onRetry}>Retry</Button>
  </p>;
}
