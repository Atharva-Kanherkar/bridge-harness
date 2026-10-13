// The quick prompt window (src-tauri/src/quick_prompt.rs): one composer,
// floating over whatever you are doing. It owns no session. Enter hands the
// text to the main window, which starts the chat the way its welcome composer
// does, and this window hides; Esc hides it without sending.

import { useEffect, useRef, useState } from "react";
import { ArrowUp } from "lucide-react";
import { BridgeGlyph } from "./BridgeGlyph";
import { modKey } from "@/lib/platform";

export const QUICK_HIDE_EVENT = "bridge-quick-hide";
export const QUICK_SUBMIT_EVENT = "bridge-quick-submit";

async function emitToShell(event: string, payload?: unknown) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const { emit } = await import("@tauri-apps/api/event");
  await emit(event, payload);
}

export function QuickPrompt({ emit = emitToShell }: { emit?: (event: string, payload?: unknown) => Promise<void> }) {
  const [text, setText] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);

  // The window is shown and hidden rather than rebuilt, so take focus again
  // every time it comes back.
  useEffect(() => {
    const focus = () => input.current?.focus();
    focus();
    window.addEventListener("focus", focus);
    return () => window.removeEventListener("focus", focus);
  }, []);

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    setText("");
    void emit(QUICK_SUBMIT_EVENT, { text: trimmed });
  };

  return <div className="flex h-screen flex-col bg-background p-4 text-foreground">
    <div className="mb-3 flex items-center gap-2 text-[12px] text-muted-foreground">
      <BridgeGlyph size={16} />
      <span>New chat in your last project</span>
      <span className="ml-auto">Esc to hide</span>
    </div>
    <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-border-card bg-card p-3 focus-within:border-ring">
      <textarea
        ref={input}
        aria-label="Ask Bridge"
        value={text}
        onChange={event => setText(event.target.value)}
        onKeyDown={event => {
          if (event.key === "Escape") { event.preventDefault(); void emit(QUICK_HIDE_EVENT); }
          else if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); submit(); }
        }}
        placeholder="Ask Bridge…"
        className="min-h-0 flex-1 resize-none bg-transparent text-message text-foreground outline-none placeholder:text-muted-foreground"
      />
      <div className="flex items-center justify-between text-[11px] text-muted-foreground">
        <span>↵ send · ⇧↵ new line · {modKey("N")} in Bridge for more options</span>
        <button
          type="button"
          aria-label="Send"
          disabled={!text.trim()}
          onClick={submit}
          className="grid size-7 place-items-center rounded-full bg-primary text-primary-foreground disabled:opacity-40"
        >
          <ArrowUp size={14} aria-hidden="true" />
        </button>
      </div>
    </div>
  </div>;
}
