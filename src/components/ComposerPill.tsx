import type { KeyboardEvent, ReactNode } from "react";
import { useEffect, useRef } from "react";
import { ArrowUp, Plus, Square } from "lucide-react";
import { cn } from "@/lib/utils";

export type ComposerPillProps = {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  placeholder?: string;
  disabled?: boolean;
  working?: boolean;
  onStop?: () => void;
  onPlusClick?: () => void;
  trailing?: ReactNode;
  className?: string;
  layout?: "hero" | "dock";
  autocomplete?: { controls: string; activeDescendant?: string };
};

export function ComposerPill({
  value,
  onChange,
  onSubmit,
  onKeyDown,
  placeholder = "Ask Bridge…",
  disabled,
  working,
  onStop,
  onPlusClick,
  trailing,
  className,
  layout = "dock",
  autocomplete,
}: ComposerPillProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const isHero = layout === "hero";
  const canSend = !disabled && !working && value.trim().length > 0;

  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(textarea.scrollHeight, isHero ? 180 : 140)}px`;
  }, [value, isHero]);

  return (
    <div className={cn("w-full", isHero ? "mx-auto max-w-2xl" : "mx-auto max-w-2xl px-3 pb-4 pt-3 sm:px-6 sm:pb-6", className)}>
      <form
        className={cn(
          "relative flex flex-col gap-1.5 rounded-[1.4rem]",
          // Resting surface: one ladder step above the canvas, no blur.
          "border border-input bg-card",
          "transition-colors duration-200",
          "focus-within:border-ring",
          isHero ? "px-4 py-3.5 sm:px-5 sm:py-4" : "px-3.5 py-2.5 sm:px-4 sm:py-3",
        )}
        onSubmit={event => {
          event.preventDefault();
          if (canSend) onSubmit();
        }}
      >
        <textarea
          ref={textareaRef}
          value={value}
          rows={1}
          placeholder={placeholder}
          disabled={disabled || working}
          role={autocomplete ? "combobox" : undefined}
          aria-autocomplete={autocomplete ? "list" : undefined}
          aria-expanded={autocomplete ? true : undefined}
          aria-controls={autocomplete?.controls}
          aria-activedescendant={autocomplete?.activeDescendant}
          onChange={event => onChange(event.target.value)}
          onKeyDown={event => {
            onKeyDown?.(event);
            if (event.defaultPrevented) return;
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              if (canSend) onSubmit();
            }
          }}
          className={cn(
            "max-h-44 min-h-[28px] w-full resize-none bg-transparent text-[15px] leading-relaxed tracking-[-0.006em] text-foreground outline-none placeholder:text-muted-foreground/70",
            isHero ? "px-1 py-1" : "px-1 py-0.5",
          )}
        />

        <div className="flex items-center justify-between gap-2 px-1">
          <button
            type="button"
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors duration-150 hover:bg-accent hover:text-foreground active:scale-95 disabled:opacity-40"
            onClick={onPlusClick}
            // Not gated on `working`: opening a workspace is a shell action, and
            // an orchestrator mid-turn is exactly when the user reaches for it.
            disabled={disabled || !onPlusClick}
            aria-label="New workspace"
            title="New workspace"
          >
            <Plus className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />
          </button>

          <div className="flex items-center gap-2">
            {trailing}
            {working && onStop ? (
              <button
                type="button"
                onClick={onStop}
                className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-input bg-card text-foreground transition-colors duration-150 active:scale-95 hover:bg-accent"
                aria-label="Stop"
              >
                <Square className="h-3.5 w-3.5 fill-current" aria-hidden="true" />
              </button>
            ) : (
              <button
                type="submit"
                disabled={!canSend}
                className={cn(
                  "inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-opacity duration-150 active:scale-95",
                  canSend
                    ? "bg-primary text-primary-foreground hover:opacity-90"
                    : "bg-accent text-muted-foreground/70",
                )}
                aria-label="Send"
              >
                <ArrowUp className="h-4 w-4" strokeWidth={2.5} aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      </form>
    </div>
  );
}
