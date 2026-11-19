import { useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { motion } from "framer-motion";
import { Check, MessageCircleQuestion, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { modKey } from "@/lib/platform";
import type { ConversationItem } from "../conversation";
import type { InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { answerSummary, canSubmit, cleanAnswers, fieldAnswered, initialAnswers, questionFields, type QuestionField, type QuestionOption } from "../questionFields";

export type ResolveQuestion = (eventId: number, action: QuestionAction, answers: Record<string, string[]>) => Promise<InteractionResolutionResult | void> | void;

const PANEL = "min-w-0 overflow-hidden rounded-xl border border-border border-x-2 border-x-info bg-card";
const WELL = "block max-h-72 overflow-auto rounded-md border border-border bg-code px-3 py-2.5 font-mono text-[12px] leading-relaxed whitespace-pre text-foreground";
const BTN_PRIMARY = "inline-flex min-h-8 items-center gap-1.5 rounded-lg bg-primary px-3 py-1 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50";
const BTN_SECONDARY = "inline-flex min-h-8 items-center gap-1.5 rounded-lg border border-input px-3 py-1 text-[13px] font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50";
const INPUT = "w-full rounded-lg border border-input bg-background px-3 py-2 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring disabled:opacity-60";

/** The typed "Other" answer, kept apart from the listed choices. */
type Other = { on: boolean; text: string };

/** Everything a harness asks: one or several questions, each a single or
 *  multiple choice (with descriptions, previews and a typed "Other"), free
 *  text, a number, a yes/no or a secret. Answers stay in the card until the
 *  person sends them; nothing here answers on their behalf. */
export function QuestionCard({ item, onResolve, resolvedCopy }: { item: ConversationItem; onResolve: ResolveQuestion; resolvedCopy: string }) {
  const fields = useMemo(() => questionFields(item.data), [item.data]);
  const [choices, setChoices] = useState<Record<string, string[]>>(() => initialAnswers(fields));
  const [others, setOthers] = useState<Record<string, Other>>({});
  const [busy, setBusy] = useState<QuestionAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState<Record<string, string[]> | null>(null);
  const transition = useMotionTransition(MOTION_DURATION.tick);
  const pending = item.status === "pending" && !sent;
  const answers = useMemo(() => combine(fields, choices, others), [fields, choices, others]);
  const ready = canSubmit(fields, answers);
  const answeredCount = fields.filter(field => fieldAnswered(field, answers[field.id])).length;
  // Digit shortcuts only when there is exactly one list to pick from, so a
  // number never lands in a field the person was not looking at.
  const shortcutField = fields.length === 1 && fields[0].kind !== "text" && fields[0].kind !== "number" && fields[0].options.length <= 9 ? fields[0] : null;
  const intro = fields.length
    ? (typeof item.data.message === "string" && item.data.message.trim() ? item.data.message : null)
    : item.text || null;

  const pick = (field: QuestionField, value: string) => {
    setChoices(current => {
      const chosen = current[field.id] ?? [];
      return {
        ...current,
        [field.id]: field.multiple
          ? chosen.includes(value) ? chosen.filter(option => option !== value) : [...chosen, value]
          : chosen.length === 1 && chosen[0] === value && !field.required ? [] : [value],
      };
    });
    // One answer per single choice: picking a listed option drops a typed one.
    if (!field.multiple) setOthers(current => current[field.id]?.on ? { ...current, [field.id]: { ...current[field.id], on: false } } : current);
  };
  const setOther = (field: QuestionField, next: Partial<Other>) => {
    setOthers(current => ({ ...current, [field.id]: { ...(current[field.id] ?? { on: false, text: "" }), ...next } }));
    if (!field.multiple && next.on) setChoices(current => ({ ...current, [field.id]: [] }));
  };
  const setText = (field: QuestionField, value: string) => setChoices(current => ({ ...current, [field.id]: [value] }));

  const act = async (action: QuestionAction) => {
    if (busy) return;
    if (action === "answer" && !ready) return;
    const payload = action === "answer" ? cleanAnswers(fields, answers) : {};
    setBusy(action);
    setError(null);
    try {
      await onResolve(item.eventId, action, payload);
      if (action === "answer") setSent(payload);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (!pending || busy) return;
    const target = event.target as HTMLElement;
    const typing = target.tagName === "INPUT" || target.tagName === "TEXTAREA";
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void act("answer");
      return;
    }
    if (typing || event.metaKey || event.ctrlKey || event.altKey || !shortcutField) return;
    const index = Number(event.key) - 1;
    const option = Number.isInteger(index) && index >= 0 ? shortcutField.options[index] : undefined;
    if (option) {
      event.preventDefault();
      pick(shortcutField, option.value);
    }
  };

  const settled = sent ?? (item.data.submittedAnswers && typeof item.data.submittedAnswers === "object" ? item.data.submittedAnswers as Record<string, string[]> : null);
  const failed = item.status === "failed" || typeof item.data.failure === "string";

  return <section aria-label="Question from the agent" onKeyDown={onKeyDown} className={PANEL}>
    <header className="flex items-center gap-2 px-3.5 pt-3 sm:px-4">
      <span aria-hidden="true" className="grid size-6 shrink-0 place-items-center rounded-full bg-info/15 text-info"><MessageCircleQuestion size={13} /></span>
      <b className="min-w-0 truncate text-[13px] font-semibold text-foreground">{fields.length > 1 ? `${fields.length} questions` : fields[0]?.header ?? (item.title || "Question")}</b>
      {pending && <span className="ml-auto inline-flex shrink-0 items-center gap-1.5 text-[11px] tracking-[0.03em] text-info"><span aria-hidden="true" className="size-1.5 rounded-full bg-info" />waiting for your answer</span>}
    </header>
    {intro && <p className="mt-1.5 px-3.5 text-[13px] leading-relaxed text-muted-foreground sm:px-4">{intro}</p>}

    {pending
        ? <div>
            {fields.length > 0
              ? <div className="divide-y divide-border/60">
                  {fields.map((field, index) => <FieldView
                    key={field.id}
                    field={field}
                    position={fields.length > 1 ? `${index + 1}/${fields.length}` : null}
                    values={choices[field.id] ?? []}
                    other={others[field.id] ?? { on: false, text: "" }}
                    disabled={!!busy}
                    shortcuts={field === shortcutField}
                    onPick={value => pick(field, value)}
                    onOther={next => setOther(field, next)}
                    onText={value => setText(field, value)}
                    onSubmit={() => void act("answer")}
                  />)}
                </div>
              : <p className="px-3.5 py-3 text-[12px] text-muted-foreground sm:px-4">This agent asked in a shape Bridge cannot answer here. Skip it, then reply in the chat.</p>}
            <footer className="flex flex-wrap items-center gap-2 border-t border-border/60 px-3.5 py-2.5 sm:px-4">
              <span className="mr-auto inline-flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                {fields.length > 1 && <span className="tabular-nums">{answeredCount} of {fields.length} answered</span>}
                {fields.length > 1 && <span aria-hidden="true">·</span>}
                <kbd className="rounded border border-border px-1 font-sans text-[10.5px]">{modKey("↵")}</kbd> to send
              </span>
              <button type="button" disabled={!!busy} className={BTN_SECONDARY} onClick={() => void act("decline")}><X size={12} aria-hidden="true" />{busy === "decline" ? "Skipping…" : "Skip"}</button>
              <button type="button" disabled={!!busy || !ready} className={BTN_PRIMARY} onClick={() => void act("answer")}><Check size={12} aria-hidden="true" />{busy === "answer" ? "Sending…" : fields.length > 1 ? "Send answers" : "Send answer"}</button>
            </footer>
          </div>
        // The settled card fades in where the form was, so it reads as the
        // question closing rather than one block replacing another.
        : <motion.div key="settled" className="px-3.5 pb-3 pt-2 sm:px-4" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} transition={transition}>
            {settled && <dl className="mb-2 space-y-1.5">
              {fields.map(field => {
                const summary = answerSummary(field, settled[field.id]);
                return summary ? <div key={field.id} className="min-w-0">
                  <dt className="text-[12px] leading-snug text-muted-foreground">{field.prompt}</dt>
                  <dd className="mt-0.5 text-[13px] font-medium leading-snug text-foreground">{summary}</dd>
                </div> : null;
              })}
            </dl>}
            <p className={cn("flex items-center gap-1.5 text-[12px]", failed ? "text-destructive" : "text-muted-foreground")}>
              {failed ? <X size={12} aria-hidden="true" /> : item.status === "answered" || sent ? <Check size={12} aria-hidden="true" /> : <X size={12} aria-hidden="true" />}
              {sent && item.status === "pending" ? "Answer sent" : resolvedCopy}
            </p>
          </motion.div>}
    {typeof item.data.failure === "string" && <p role="alert" className="px-3.5 pb-3 text-[12px] text-destructive sm:px-4">{item.data.failure}</p>}
    {error && <p role="alert" className="px-3.5 pb-3 text-[12px] text-destructive sm:px-4">{error}</p>}
  </section>;
}

/** Listed choices plus the typed "Other", as the provider will receive them. */
function combine(fields: readonly QuestionField[], choices: Record<string, string[]>, others: Record<string, Other>): Record<string, string[]> {
  return Object.fromEntries(fields.map(field => {
    const other = others[field.id];
    const typed = field.allowOther && other?.on && other.text.trim() ? [other.text] : [];
    return [field.id, [...(choices[field.id] ?? []), ...typed]];
  }));
}

function FieldView({ field, position, values, other, disabled, shortcuts, onPick, onOther, onText, onSubmit }: {
  field: QuestionField;
  position: string | null;
  values: string[];
  other: Other;
  disabled: boolean;
  shortcuts: boolean;
  onPick: (value: string) => void;
  onOther: (next: Partial<Other>) => void;
  onText: (value: string) => void;
  onSubmit: () => void;
}) {
  const promptId = useId();
  const otherInput = useRef<HTMLInputElement>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  const choice = field.kind === "choice" || field.kind === "boolean";
  const previewOption = field.options.find(option => option.value === previewing) ?? field.options.find(option => values.includes(option.value) && option.preview);
  const hasPreviews = field.options.some(option => option.preview);
  return <div role="group" aria-labelledby={promptId} className="px-3.5 py-3 sm:px-4">
    {(position || field.header || field.multiple) && <div className="mb-1.5 flex flex-wrap items-center gap-1.5">
      {position && <span className="text-[11px] tabular-nums text-muted-foreground">{position}</span>}
      {field.header && <span className="rounded-md border border-border bg-muted px-1.5 py-px text-[10.5px] font-medium uppercase tracking-[0.04em] text-muted-foreground">{field.header}</span>}
      {field.multiple && choice && <span className="text-[11px] text-muted-foreground">Choose any</span>}
    </div>}
    <p id={promptId} className="text-[14px] font-medium leading-snug text-foreground">{field.prompt}</p>
    {field.hint && <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{field.hint}</p>}

    {choice
      ? <div role={field.multiple ? "group" : "radiogroup"} aria-labelledby={promptId} className="mt-2.5 grid gap-1.5">
          {field.options.map((option, index) => <OptionRow
            key={option.value}
            option={option}
            multiple={field.multiple}
            selected={values.includes(option.value)}
            disabled={disabled}
            shortcut={shortcuts ? String(index + 1) : null}
            onSelect={() => onPick(option.value)}
            onPreview={hasPreviews ? () => setPreviewing(option.value) : undefined}
          />)}
          {field.allowOther && <div
            onClick={event => { if (event.target === event.currentTarget) otherInput.current?.focus(); }}
            className={cn(
              "flex w-full cursor-text items-center gap-3 rounded-lg border px-3 py-1.5 transition-colors",
              other.on ? "border-info/60 bg-info/10" : "border-dashed border-border hover:border-foreground/25",
            )}
          >
            <button
              type="button"
              role={field.multiple ? "checkbox" : "radio"}
              aria-checked={other.on}
              aria-label="Something else"
              disabled={disabled}
              onClick={() => { const on = !other.on; onOther({ on }); if (on) otherInput.current?.focus(); }}
              className="shrink-0 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
            ><Indicator multiple={field.multiple} selected={other.on} /></button>
            <input
              ref={otherInput}
              aria-label={`${field.prompt}: something else`}
              disabled={disabled}
              value={other.text}
              onChange={event => onOther({ text: event.target.value, on: event.target.value.trim().length > 0 || other.on })}
              onKeyDown={event => { if (event.key === "Enter" && !event.metaKey && !event.ctrlKey) { event.preventDefault(); onSubmit(); } }}
              placeholder="Something else…"
              className="min-h-7 w-full bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
            />
          </div>}
          {previewOption?.preview && <div className="mt-1">
            <small className="mb-1 block text-[10.5px] font-medium uppercase tracking-[0.04em] text-muted-foreground">Preview · {previewOption.label}</small>
            <pre className={WELL}>{previewOption.preview}</pre>
          </div>}
        </div>
      : <div className="mt-2.5">
          {field.kind === "text" && !field.secret
            ? <textarea
                aria-labelledby={promptId}
                rows={2}
                disabled={disabled}
                value={values[0] ?? ""}
                onChange={event => onText(event.target.value)}
                onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey && !event.metaKey && !event.ctrlKey) { event.preventDefault(); onSubmit(); } }}
                placeholder="Type your answer"
                className={cn(INPUT, "min-h-[4.25rem] resize-y leading-relaxed")}
              />
            : <input
                aria-labelledby={promptId}
                type={field.secret ? "password" : field.kind === "number" ? "number" : "text"}
                inputMode={field.kind === "number" ? (field.integer ? "numeric" : "decimal") : undefined}
                step={field.kind === "number" ? (field.integer ? 1 : "any") : undefined}
                autoComplete="off"
                disabled={disabled}
                value={values[0] ?? ""}
                onChange={event => onText(event.target.value)}
                onKeyDown={event => { if (event.key === "Enter" && !event.metaKey && !event.ctrlKey) { event.preventDefault(); onSubmit(); } }}
                placeholder={field.secret ? "Hidden after you send it" : field.kind === "number" ? "Enter a number" : "Type your answer"}
                className={INPUT}
              />}
          {!field.required && <small className="mt-1 block text-[11px] text-muted-foreground">Optional</small>}
        </div>}
  </div>;
}

function OptionRow({ option, multiple, selected, disabled, shortcut, onSelect, onPreview }: {
  option: QuestionOption;
  multiple: boolean;
  selected: boolean;
  disabled: boolean;
  shortcut: string | null;
  onSelect: () => void;
  onPreview?: () => void;
}) {
  return <button
    type="button"
    role={multiple ? "checkbox" : "radio"}
    aria-checked={selected}
    disabled={disabled}
    onClick={onSelect}
    onMouseEnter={onPreview}
    onFocus={onPreview}
    className={cn(
      "flex w-full items-start gap-3 rounded-lg border px-3 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60",
      selected ? "border-info/60 bg-info/10" : "border-border hover:border-foreground/25 hover:bg-accent",
    )}
  >
    <span className="mt-0.5"><Indicator multiple={multiple} selected={selected} /></span>
    <span className="min-w-0 flex-1">
      <span className="block text-[13px] font-medium leading-snug text-foreground">{option.label}</span>
      {option.description && <span className="mt-0.5 block text-[12px] leading-relaxed text-muted-foreground">{option.description}</span>}
    </span>
    {shortcut && <kbd aria-hidden="true" className="mt-0.5 shrink-0 rounded border border-border px-1.5 font-mono text-[10.5px] text-muted-foreground">{shortcut}</kbd>}
  </button>;
}

function Indicator({ multiple, selected }: { multiple: boolean; selected: boolean }) {
  return <span aria-hidden="true" className={cn(
    "grid size-4 shrink-0 place-items-center border transition-colors",
    multiple ? "rounded-[4px]" : "rounded-full",
    selected ? "border-info bg-info text-background" : "border-input bg-background",
  )}>{selected && (multiple ? <Check size={10} strokeWidth={3} /> : <span className="size-1.5 rounded-full bg-background" />)}</span>;
}
