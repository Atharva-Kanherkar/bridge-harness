import { useEffect, useMemo, useState, type FormEvent } from "react";
import { ArrowUp, Check, Route } from "lucide-react";
import type { PreviewTrailBriefResult, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { briefTotal, chosenInOrder, defaultBriefSelection, formatTokens } from "@/trails";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { HarnessMark } from "@/components/harnessMarks";
import { SkeletonRows } from "@/components/ui/skeleton";
import { clsx as cn } from "clsx";

export type TrailHarnessOption = { id: string; label: string };

export type TrailStartRequest = { trail: TrailSummary; text: string; harness: string; items: string[] };

export function TrailStartDialog({ trail, harnesses, defaultHarness, initialText = "", onStart, onClose }: {
  trail: TrailSummary;
  harnesses: TrailHarnessOption[];
  defaultHarness?: string | null;
  initialText?: string;
  onStart: (request: TrailStartRequest) => Promise<void>;
  onClose: () => void;
}) {
  const [preview, setPreview] = useState<PreviewTrailBriefResult | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [text, setText] = useState(initialText);
  const [harness, setHarness] = useState(defaultHarness && harnesses.some(item => item.id === defaultHarness) ? defaultHarness : harnesses[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let live = true;
    bridgeApi.previewTrailBrief(trail.id)
      .then(result => { if (!live) return; setPreview(result); setChosen(defaultBriefSelection(result.items)); })
      .catch(cause => { if (live) setError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { live = false; };
  }, [trail.id]);

  const total = useMemo(() => preview ? briefTotal(preview.items, chosen) : 0, [preview, chosen]);
  const over = !!preview && total > preview.budgetTokens;
  const canSend = !!text.trim() && !!harness && !!preview && !over && !busy;

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    if (!canSend || !preview) return;
    setBusy(true); setError(undefined);
    try {
      await onStart({ trail, text: text.trim(), harness, items: chosenInOrder(preview.items, chosen) });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setBusy(false);
    }
  }

  function toggle(id: string) {
    setChosen(current => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }

  return (
    <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
      <DialogPopup aria-label={`New chat on ${trail.title}`} className="max-w-xl">
        <form onSubmit={submit} className="flex min-h-0 flex-col gap-4 p-5">
          <div className="flex items-center gap-2 pr-8">
            <Route className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <h2 className="truncate text-ui font-medium text-foreground">New chat on “{trail.title}”</h2>
          </div>
          <div className="rounded-xl bg-card p-3">
            <textarea
              autoFocus
              value={text}
              onChange={event => setText(event.target.value)}
              onKeyDown={event => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) void submit(); }}
              rows={3}
              placeholder="What should this chat do next?"
              aria-label="First message"
              className="w-full resize-none bg-transparent text-message text-foreground outline-none placeholder:text-faint"
            />
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <div role="radiogroup" aria-label="Agent" className="flex flex-wrap gap-1">
                {harnesses.map(option => (
                  <button
                    key={option.id}
                    type="button"
                    role="radio"
                    aria-checked={harness === option.id}
                    onClick={() => setHarness(option.id)}
                    className={cn(
                      "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-caption font-medium transition-colors",
                      harness === option.id ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/60",
                    )}
                  >
                    <HarnessMark harness={option.id} size={12} />{option.label}
                  </button>
                ))}
              </div>
              <span className="flex-1" />
              <button
                type="submit"
                disabled={!canSend}
                aria-label="Start chat"
                className="flex size-7 items-center justify-center rounded-full bg-primary text-primary-foreground disabled:opacity-40"
              >
                <ArrowUp className="size-3.5" aria-hidden="true" />
              </button>
            </div>
          </div>

          <div className="min-h-0">
            <div className="mb-1 flex items-baseline justify-between gap-2">
              <h3 className="text-caption font-medium uppercase tracking-wide text-faint">This chat starts with</h3>
              {preview && (
                <span className={cn("font-mono text-caption", over ? "text-destructive" : "text-faint")} data-testid="brief-total">
                  {formatTokens(total)} of {formatTokens(preview.budgetTokens)} tokens
                </span>
              )}
            </div>
            {!preview && !error && <SkeletonRows label="Loading what the chat starts with" rows={3} framed={false} />}
            {preview && preview.items.length === 0 && (
              <p className="py-2 text-ui text-muted-foreground">Nothing to carry yet. The chat joins the trail and starts fresh.</p>
            )}
            {preview && preview.items.length > 0 && (
              <ul className="max-h-64 divide-y divide-border overflow-y-auto">
                {preview.items.map(item => {
                  const on = chosen.has(item.id);
                  return (
                    <li key={item.id}>
                      <label className="flex cursor-default items-center gap-2.5 py-1.5">
                        <input type="checkbox" checked={on} onChange={() => toggle(item.id)} className="peer sr-only" aria-label={item.label} />
                        <span aria-hidden="true" className={cn("flex size-4 shrink-0 items-center justify-center rounded peer-focus-visible:ring-2 peer-focus-visible:ring-ring", on ? "bg-primary text-primary-foreground" : "bg-muted")}>
                          {on && <Check className="size-3" />}
                        </span>
                        <span className={cn("min-w-0 flex-1 truncate text-ui", on ? "text-foreground" : "text-muted-foreground")}>{item.label}</span>
                        <span className="shrink-0 font-mono text-caption text-faint">{formatTokens(item.tokens)}</span>
                      </label>
                    </li>
                  );
                })}
              </ul>
            )}
            {over && <p className="mt-2 text-caption text-destructive">That is more than a new chat can start with. Untick some items.</p>}
            {preview && !over && preview.items.length > 0 && (
              <p className="mt-2 text-caption text-muted-foreground">The agent reads these before your first message, whichever agent you pick. Nothing else about the chat changes.</p>
            )}
            {error && <p role="alert" className="mt-2 text-caption text-destructive">{error}</p>}
          </div>
        </form>
      </DialogPopup>
    </Dialog>
  );
}
