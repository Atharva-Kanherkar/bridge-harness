import { useEffect, useMemo, useState, type FormEvent } from "react";
import { ArrowUp, Route } from "lucide-react";
import type { PreviewTrailBriefResult, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { friendlyError } from "@/errors";
import { briefTotal, chosenInOrder, defaultBriefSelection, formatTokens } from "@/trails";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { HarnessMark } from "@/components/harnessMarks";
import { SkeletonRows } from "@/components/ui/skeleton";
import { clsx as cn } from "clsx";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { SegmentedRadio } from "@/components/ui/segmented";
import { Checkbox } from "@/components/ui/checkbox";

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
      .catch(cause => { if (live) setError(friendlyError(cause, "Couldn't load what the chat starts with.")); });
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
      const reason = friendlyError(cause, "Couldn't start the chat.");
      // Nothing was sent. Re-read what the chat can start with, untick what
      // left the trail and say which, and keep the text for another try.
      const fresh = await bridgeApi.previewTrailBrief(trail.id).catch(() => null);
      if (fresh) {
        const gone = preview.items.filter(item => chosen.has(item.id) && !fresh.items.some(next => next.id === item.id));
        setPreview(fresh);
        setChosen(current => new Set([...current].filter(id => fresh.items.some(next => next.id === id))));
        setError(gone.length
          ? `${gone.map(item => `“${item.label.replace(/^\S+ “|”.*$/g, "")}”`).join(", ")} ${gone.length === 1 ? "is" : "are"} no longer on this trail, so ${gone.length === 1 ? "it was" : "they were"} unticked. Check the list and send again.`
          : `${reason} Nothing was sent.`);
      } else {
        setError(`${reason} Nothing was sent.`);
      }
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
          <div className="rounded-xl bg-card p-3 text-message">
            <Textarea
              unstyled
              size="bare"
              autoFocus
              value={text}
              onChange={event => setText(event.target.value)}
              onKeyDown={event => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) void submit(); }}
              rows={3}
              placeholder="What should this chat do next?"
              aria-label="First message"
              className="w-full"
            />
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <SegmentedRadio
                label="Agent"
                value={harness}
                options={harnesses.map(option => ({ value: option.id, label: <><HarnessMark harness={option.id} size={12} />{option.label}</> }))}
                onChange={setHarness}
              />
              <span className="flex-1" />
              <Button variant="default" size="icon-xs" type="submit"
                disabled={!canSend}
                aria-label="Start chat"
              >
                <ArrowUp className="size-3.5" aria-hidden="true" />
              </Button>
            </div>
          </div>

          <div className="min-h-0">
            <div className="mb-1 flex items-baseline justify-between gap-2">
              <h3 className="text-caption font-medium text-faint">This chat starts with</h3>
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
                        <Checkbox checked={on} onCheckedChange={() => toggle(item.id)} aria-label={item.label} />
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
