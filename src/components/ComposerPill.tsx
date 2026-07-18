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
    <div className={cn("w-full", isHero ? "mx-auto max-w-2xl" : "mx-auto max-w-2xl px-4 pb-6 pt-3 sm:px-6", className)}>
      <form
        className={cn(
          "relative flex flex-col gap-1.5 rounded-[1.4rem]",
          "border border-white/[0.08] bg-white/[0.045] backdrop-blur-2xl backdrop-saturate-[1.8]",
          "shadow-[0_18px_50px_-20px_rgba(0,0,0,0.65),inset_0_1px_0_rgba(255,255,255,0.07)]",
          "transition-all duration-500",
          "focus-within:border-white/[0.16] focus-within:bg-white/[0.06] focus-within:shadow-[0_18px_50px_-16px_rgba(0,0,0,0.7),0_0_40px_-8px_rgba(148,163,253,0.14),inset_0_1px_0_rgba(255,255,255,0.09)]",
          isHero ? "px-5 py-4" : "px-4 py-3",
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
            "max-h-44 min-h-[28px] w-full resize-none bg-transparent text-[15px] leading-relaxed tracking-[-0.006em] text-neutral-100 outline-none placeholder:text-neutral-600",
            isHero ? "px-1 py-1" : "px-1 py-0.5",
          )}
        />

        <div className="flex items-center justify-between gap-2 px-1">
          <button
            type="button"
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-neutral-500 transition-all duration-300 hover:bg-white/[0.08] hover:text-neutral-100 active:scale-95 disabled:opacity-40"
            onClick={onPlusClick}
            disabled={disabled || working || !onPlusClick}
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
                className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-white/[0.1] bg-white/[0.07] text-neutral-200 transition-all duration-300 active:scale-95 hover:bg-white/[0.12]"
                aria-label="Stop"
              >
                <Square className="h-3.5 w-3.5 fill-current" aria-hidden="true" />
              </button>
            ) : (
              <button
                type="submit"
                disabled={!canSend}
                className={cn(
                  "inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-all duration-300 active:scale-95",
                  canSend
                    ? "bg-white text-[#0a0a0c] shadow-[0_6px_20px_-6px_rgba(255,255,255,0.4)] hover:scale-105"
                    : "bg-white/[0.06] text-neutral-600",
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
