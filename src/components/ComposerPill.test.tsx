// @vitest-environment jsdom
import { act, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ComposerPill, type ComposerPillProps } from "./ComposerPill";

// The composer is the user's only steering wheel over a working agent, so the
// coverage here is about what its controls *do*, not how they look: the `+`
// performs the action its label names, and a draft is never collateral damage.

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  act(() => {
    root = createRoot(container);
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const props = (overrides: Partial<ComposerPillProps> = {}): ComposerPillProps => ({
  value: "",
  onChange: () => {},
  onSubmit: () => {},
  ...overrides,
});

function render(overrides: Partial<ComposerPillProps> = {}) {
  act(() => root.render(<ComposerPill {...props(overrides)} />));
}

const plus = () => container.querySelector<HTMLButtonElement>('button[aria-label="Attach a file"]')!;
const textarea = () => container.querySelector<HTMLTextAreaElement>("textarea")!;
const stop = () => container.querySelector<HTMLButtonElement>('button[aria-label="Stop"]');

describe("ComposerPill", () => {
  it("runs the named + action and leaves a non-empty draft alone", () => {
    const onPlusClick = vi.fn();
    const onChange = vi.fn();
    render({ value: "keep this draft", onPlusClick, onChange });

    act(() => plus().click());

    expect(onPlusClick).toHaveBeenCalledTimes(1);
    // A control labelled "New workspace" must not double as a draft eraser.
    expect(onChange).not.toHaveBeenCalled();
    expect(textarea().value).toBe("keep this draft");
  });

  it("keeps + reachable while the agent is working", () => {
    const onPlusClick = vi.fn();
    render({ value: "draft", working: true, onPlusClick, onStop: () => {} });

    expect(plus().disabled).toBe(false);
    act(() => plus().click());
    expect(onPlusClick).toHaveBeenCalledTimes(1);
  });

  it("disables + only when the composer itself is disabled or has no handler", () => {
    render({ onPlusClick: () => {}, disabled: true });
    expect(plus().disabled).toBe(true);

    render({});
    expect(plus().disabled).toBe(true);
  });

  it("says what + does on this surface rather than assuming", () => {
    render({ onPlusClick: () => {} });
    // The default is the common case: adding context to a conversation.
    expect(plus().title).toBe("Attach a file");

    render({ onPlusClick: () => {}, plusLabel: "New workspace" });
    const structural = container.querySelector<HTMLButtonElement>('button[aria-label="New workspace"]')!;
    expect(structural).not.toBeNull();
    expect(structural.disabled).toBe(false);
  });

  it("explains an unavailable + instead of leaving a dead control", () => {
    render({ onPlusClick: () => {}, plusUnavailableReason: "Connect a folder to this chat to attach files from it" });
    expect(plus().disabled).toBe(true);
    expect(plus().title).toBe("Connect a folder to this chat to attach files from it");
  });

  it("stays editable while working, with Steer and Stop both reachable", () => {
    const onSubmit = vi.fn();
    const onStop = vi.fn();
    render({ value: "use the other API", working: true, activeAction: "steer", onSubmit, onStop });

    // The whole point: a working agent is when supervision is worth the most.
    expect(textarea().disabled).toBe(false);
    const submit = container.querySelector<HTMLButtonElement>('button[aria-label="Steer"]')!;
    expect(submit.disabled).toBe(false);
    expect(stop()).not.toBeNull();

    act(() => submit.click());
    expect(onSubmit).toHaveBeenCalledTimes(1);
    // Sending guidance must never read as cancelling the work.
    expect(onStop).not.toHaveBeenCalled();
  });

  it("says Queue when the provider cannot take input mid-turn", () => {
    render({ value: "also update the docs", working: true, activeAction: "queue", onStop: () => {} });

    const submit = container.querySelector<HTMLButtonElement>('button[aria-label="Queue"]')!;
    expect(submit.disabled).toBe(false);
    expect(submit.title).toBe("Held until the current step finishes");
    expect(container.querySelector('button[aria-label="Steer"]')).toBeNull();
  });

  it("submits on Enter during an active turn", () => {
    const onSubmit = vi.fn();
    render({ value: "steer me", working: true, activeAction: "steer", onSubmit, onStop: () => {} });

    act(() => {
      textarea().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("locks the composer during a turn only where nothing can be submitted", () => {
    // A surface with no active-turn action keeps the old read-only behaviour, so
    // dropping `activeAction` cannot silently promise steering.
    render({ value: "draft", working: true, onStop: () => {} });

    expect(textarea().disabled).toBe(true);
    expect(container.querySelector('button[aria-label="Send"]')).toBeNull();
    expect(stop()).not.toBeNull();
  });

  it("refuses an empty submission whatever the turn is doing", () => {
    const onSubmit = vi.fn();
    render({ value: "   ", working: true, activeAction: "steer", onSubmit, onStop: () => {} });

    const submit = container.querySelector<HTMLButtonElement>('button[aria-label="Steer"]')!;
    expect(submit.disabled).toBe(true);
    act(() => submit.click());
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps Stop reachable while working", () => {
    const onStop = vi.fn();
    render({ value: "draft", working: true, onStop });

    act(() => stop()!.click());
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  describe("inline suggestions", () => {
    const putCaretAtEnd = (value: string) => {
      act(() => {
        textarea().setSelectionRange(value.length, value.length);
        textarea().dispatchEvent(new Event("select", { bubbles: true }));
      });
    };

    it("renders ghost text after the draft when the caret is at the end", () => {
      render({ value: "let's ship the", suggestion: " release" });
      putCaretAtEnd("let's ship the");
      // A re-render is needed for the caret-at-end check to observe the
      // selection set above — mirrors how a real keyup/mouseup would.
      render({ value: "let's ship the", suggestion: " release" });

      expect(container.textContent).toContain("release");
    });

    it("hides the ghost text once the caret has moved away from the end", () => {
      render({ value: "let's ship the release", suggestion: " notes" });
      act(() => {
        textarea().setSelectionRange(0, 0);
        textarea().dispatchEvent(new Event("select", { bubbles: true }));
      });

      // "notes" must not appear as ghost text once the caret is no longer
      // trailing the draft — the continuation would land in the wrong place.
      expect(container.textContent).not.toContain("notes");
    });

    it("accepts the suggestion on Tab and prevents the browser's own Tab behavior", () => {
      const onAcceptSuggestion = vi.fn();
      render({ value: "draft", suggestion: " continues", onAcceptSuggestion });
      putCaretAtEnd("draft");
      render({ value: "draft", suggestion: " continues", onAcceptSuggestion });

      const event = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event); });

      expect(onAcceptSuggestion).toHaveBeenCalledTimes(1);
      expect(event.defaultPrevented).toBe(true);
    });

    it("yields Tab to a mention/slash popover that already handled it", () => {
      const onAcceptSuggestion = vi.fn();
      // The composer's own onKeyDown mirrors what App.tsx does for an open
      // popover: it claims the key by calling preventDefault first.
      const onKeyDown = (event: ReactKeyboardEvent<HTMLTextAreaElement>) => event.preventDefault();
      render({ value: "@fi", suggestion: " le.ts", onAcceptSuggestion, onKeyDown });
      putCaretAtEnd("@fi");
      render({ value: "@fi", suggestion: " le.ts", onAcceptSuggestion, onKeyDown });

      act(() => {
        textarea().dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      });

      expect(onAcceptSuggestion).not.toHaveBeenCalled();
    });

    it("does not accept Tab after the caret moves away, even before a re-render", () => {
      const onAcceptSuggestion = vi.fn();
      render({ value: "draft", suggestion: " continues", onAcceptSuggestion });
      putCaretAtEnd("draft");

      act(() => textarea().setSelectionRange(0, 0));
      const event = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event); });

      expect(onAcceptSuggestion).not.toHaveBeenCalled();
      expect(event.defaultPrevented).toBe(false);
    });

    it("does nothing on Tab when there is no suggestion to accept", () => {
      const onAcceptSuggestion = vi.fn();
      render({ value: "draft", onAcceptSuggestion });

      const event = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event); });

      expect(onAcceptSuggestion).not.toHaveBeenCalled();
      expect(event.defaultPrevented).toBe(false);
    });
  });

  describe("image attachments", () => {
    const chip = () => container.querySelector<HTMLButtonElement>('button[aria-label="Remove attached image"]');

    it("hands the paste to the owner before the textarea can insert anything", () => {
      const onPaste = vi.fn();
      render({ value: "draft", onPaste });

      // jsdom has no ClipboardEvent constructor; a plain paste event carries
      // everything the forwarding contract needs.
      const event = new Event("paste", { bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event as unknown as ClipboardEvent); });

      expect(onPaste).toHaveBeenCalledTimes(1);
    });

    it("renders one removable chip per attachment, with its own id", () => {
      const onRemoveAttachment = vi.fn();
      render({
        value: "",
        attachments: [
          { id: "a", mediaType: "image/png", dataUri: "data:image/png;base64,AAA" },
          { id: "b", mediaType: "image/jpeg", dataUri: "data:image/jpeg;base64,BBB" },
        ],
        onRemoveAttachment,
      });

      expect(container.querySelectorAll("img")).toHaveLength(2);
      const removeButtons = container.querySelectorAll<HTMLButtonElement>('button[aria-label="Remove attached image"]');
      expect(removeButtons).toHaveLength(2);
      act(() => removeButtons[1].click());
      expect(onRemoveAttachment).toHaveBeenCalledWith("b");
    });

    it("lets Enter send when the message is only an image", () => {
      const onSubmit = vi.fn();
      render({ value: "", attachments: [{ id: "a", mediaType: "image/png", dataUri: "data:image/png;base64,AAA" }], onSubmit });

      const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event); });

      expect(onSubmit).toHaveBeenCalledTimes(1);
    });

    it("keeps an image-only send blocked while the composer is locked", () => {
      const onSubmit = vi.fn();
      render({ value: "", disabled: true, attachments: [{ id: "a", mediaType: "image/png", dataUri: "data:image/png;base64,AAA" }], onSubmit });

      const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
      act(() => { textarea().dispatchEvent(event); });

      expect(onSubmit).not.toHaveBeenCalled();
    });
  });
});
