import { Button } from "@/components/ui/button";
import { Tip } from "@/components/ui/tooltip";
export function ComposerSuggestionStatus({ error, onRetry }: { error?: string; onRetry: () => void }) {
  if (!error) return null;
  return <Tip label={error}><p role="status" className="mt-2 text-xs text-muted-foreground">
    Inline suggestions unavailable. Check Settings → Composer.{" "}
    <Button variant="link" size="inline" onClick={onRetry}>Retry</Button>
  </p></Tip>;
}
