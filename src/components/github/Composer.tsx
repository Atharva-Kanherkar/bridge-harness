import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Send } from "lucide-react";
import { modKey } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

/** A write that reports back: `undefined` when GitHub took it, otherwise the
 *  reason it did not, verbatim. */
export type InlineWrite = (body: string) => Promise<string | undefined>;

/**
 * The pane's one text composer: a conversation comment, a reply inside a
 * review thread, a new comment on a diff line.
 *
 * Its button is the confirmation. A comment is something the reader typed and
 * then chose to send, the button names what it does, and it can be edited or
 * deleted on GitHub afterwards, so it does not get a second sheet the way a
 * merge or a close does. A refused write keeps the draft and says why.
 */
export function Composer({ label, placeholder, submitLabel, onSubmit, onCancel, collapsed = false, autoFocus = false, disabled = false, className }: {
  /** The accessible name of the text field. */
  label: string;
  placeholder: string;
  submitLabel: string;
  onSubmit: InlineWrite;
  /** Present on a composer that can be dismissed (a diff line's). */
  onCancel?: () => void;
  /** Start as one quiet line that opens on focus (a thread's reply box). */
  collapsed?: boolean;
  autoFocus?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const reduced = useReducedMotion();
  const [body, setBody] = useState("");
  const [open, setOpen] = useState(!collapsed);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const field = useRef<HTMLTextAreaElement>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => { if (open && (autoFocus || collapsed)) field.current?.focus(); }, [open, autoFocus, collapsed]);

  const ready = body.trim().length > 0 && !busy && !disabled;
  const send = async () => {
    if (!ready) return;
    setBusy(true); setError(undefined);
    const failure = await onSubmit(body.trim());
    if (!alive.current) return;
    setBusy(false);
    if (failure !== undefined) { setError(failure); return; }
    setBody("");
    if (collapsed) setOpen(false);
    onCancel?.();
  };
  const dismiss = () => {
    setError(undefined);
    if (collapsed) { setOpen(false); setBody(""); }
    onCancel?.();
  };

  if (!open) return <Button
    variant="outline"
    size="sm"
    disabled={disabled}
    onClick={() => setOpen(true)}
    className={cn("w-full justify-start", className)}
  ><span className="truncate font-normal text-muted-foreground">{placeholder}</span></Button>;

  return <motion.div
    initial={reduced || !collapsed ? false : { opacity: 0, y: -4 }}
    animate={{ opacity: 1, y: 0 }}
    transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
    className={cn("rounded-lg border border-border bg-card p-2 text-meta leading-relaxed transition-[border-color,box-shadow] focus-within:border-foreground/30 focus-within:shadow-xs", className)}
  >
    <Textarea
      unstyled
      size="bare"
      ref={field}
      value={body}
      disabled={busy}
      onChange={event => setBody(event.target.value)}
      onKeyDown={event => {
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); event.stopPropagation(); void send(); }
        else if (event.key === "Escape" && (collapsed || onCancel)) { event.preventDefault(); event.stopPropagation(); dismiss(); }
      }}
      aria-label={label}
      placeholder={placeholder}
      className="block min-h-16 w-full"
    />
    <AnimatePresence initial={false}>
      {error !== undefined && <motion.p
        role="alert"
        initial={reduced ? false : { opacity: 0, height: 0 }}
        animate={{ opacity: 1, height: "auto" }}
        exit={reduced ? undefined : { opacity: 0, height: 0 }}
        className="overflow-hidden rounded-md bg-destructive/10 px-2 py-1 text-caption text-destructive"
      >{error}</motion.p>}
    </AnimatePresence>
    <div className="mt-1 flex items-center justify-end gap-1.5">
      <span className="mr-auto flex items-center gap-1 text-caption text-muted-foreground">Markdown · <Kbd>{modKey("↵")}</Kbd></span>
      {(collapsed || onCancel) && <Button variant="quiet" size="xs" disabled={busy} onClick={dismiss}>Cancel</Button>}
      <Button variant="default" size="xs"
        disabled={!ready}
        loading={busy}
        onClick={() => void send()}
      >
        <Send size={11} aria-hidden="true" />
        {submitLabel}
      </Button>
    </div>
  </motion.div>;
}
