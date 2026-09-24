// @vitest-environment jsdom
import { act, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ComposerPill, type ComposerPillProps } from "./ComposerPill";

// The composer is the user's only steering wheel over a working agent, so the
// coverage here is about what its controls *do*, not how they look: attaching
// is one control, and a draft is never collateral damage.

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

const attach = () => container.querySelector<HTMLButtonElement>('button[aria-label="Attach images"]');
const textarea = () => container.querySelector<HTMLTextAreaElement>("textarea")!;
const stop = () => container.querySelector<HTMLButtonElement>('button[aria-label="Stop"]');

describe("ComposerPill", () => {
  it("uploads selected images without changing the draft", () => {
    const onAttachFiles = vi.fn();
    const onChange = vi.fn();
    render({ value: "describe these", onAttachFiles, onChange });
    const file = new File(["png"], "test.png", { type: "image/png" });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(input, "files", { value: [file] });
    act(() => input.dispatchEvent(new Event("change", { bubbles: true })));
    expect(onAttachFiles).toHaveBeenCalledWith([file]);
    expect(onChange).not.toHaveBeenCalled();
    expect(textarea().value).toBe("describe these");
    expect(input.value).toBe("");
  });

  // A second, generic attach control used to sit beside the paperclip: one
  // surface showed two paperclips, the others a paperclip and a bare `+`. The
  // attach affordance is singular now, and only present when the surface takes
  // attachments at all.
  it("offers exactly one attach control, and none without a handler", () => {
    render({ onAttachFiles: () => {} });
    expect(container.querySelectorAll('button[aria-label="Attach images"]').length).toBe(1);

    render({});
    expect(attach()).toBeNull();
  });

  it("locks attaching only while the composer itself is locked", () => {
    render({ onAttachFiles: () => {} });
    expect(attach()!.disabled).toBe(false);

    render({ onAttachFiles: () => {}, disabled: true });
    expect(attach()!.disabled).toBe(true);
  });

  it("renders a leading control beside the attach button", () => {
    render({
      onAttachFiles: () => {},
      leading: <button type="button" aria-label="Open usage health details">ring</button>,
    });
    expect(attach()!.nextElementSibling?.getAttribute("aria-label")).toBe("Open usage health details");
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

  it("starts dictation on key press and stops it on release", () => {
    const onVoiceStart = vi.fn();
    const onVoiceStop = vi.fn();
    render({ voiceAvailable: true, onVoiceStart, onVoiceStop });
    const mic = container.querySelector<HTMLButtonElement>('button[aria-label="Hold to dictate"]')!;

    act(() => mic.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
    act(() => mic.dispatchEvent(new KeyboardEvent("keyup", { key: " ", bubbles: true })));

    expect(onVoiceStart).toHaveBeenCalledTimes(1);
    expect(onVoiceStop).toHaveBeenCalledTimes(1);
  });

  it("keeps an unavailable mic discoverable with its actual reason", () => {
    const onVoiceStart = vi.fn();
    render({ voiceAvailable: false, voiceUnavailableReason: "Start the Codex chat before dictating", onVoiceStart, onVoiceStop: vi.fn() });
    const mic = container.querySelector<HTMLButtonElement>('button[aria-label="Hold to dictate"]')!;
    expect(mic).not.toBeNull();
    expect(mic.disabled).toBe(true);
    expect(mic.title).toBe("Start the Codex chat before dictating");
    act(() => mic.click());
    expect(onVoiceStart).not.toHaveBeenCalled();
  });

  it.each(["starting", "recording", "stopping"] as const)("protects the draft and blocks Send while %s", voiceState => {
    const onSubmit = vi.fn();
    render({ value: "original draft", voiceAvailable: true, voiceState, voicePreview: "spoken preview", onSubmit, onAttachFiles: vi.fn(), onVoiceStart: vi.fn(), onVoiceStop: vi.fn(), onVoiceCancel: vi.fn() });
    expect(textarea().value).toBe("original draft");
    expect(textarea().readOnly).toBe(true);
    expect(attach()!.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="Send"]')!.disabled).toBe(true);
    expect(container.querySelector('[role="status"]')!.textContent).toContain("spoken preview");
    expect(container.querySelector('button[aria-label="Cancel dictation"]')).not.toBeNull();
    act(() => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("Enter finishes recording without sending and Escape cancels", () => {
    const onSubmit = vi.fn();
    const onVoiceStop = vi.fn();
    const onVoiceCancel = vi.fn();
    render({ value: "draft", voiceAvailable: true, voiceState: "recording", onSubmit, onVoiceStart: vi.fn(), onVoiceStop, onVoiceCancel });
    act(() => textarea().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onVoiceStop).toHaveBeenCalledTimes(1);
    expect(onSubmit).not.toHaveBeenCalled();
    act(() => textarea().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(onVoiceCancel).toHaveBeenCalledTimes(1);
  });

  it("accepts assistive-technology activation as a toggle", () => {
    const onVoiceStart = vi.fn();
    const onVoiceStop = vi.fn();
    render({ voiceAvailable: true, onVoiceStart, onVoiceStop });
    act(() => container.querySelector<HTMLButtonElement>('button[aria-label="Hold to dictate"]')!.click());
    expect(onVoiceStart).toHaveBeenCalledTimes(1);
    render({ voiceAvailable: true, voiceState: "recording", onVoiceStart, onVoiceStop });
    act(() => container.querySelector<HTMLButtonElement>('button[aria-label="Stop dictating"]')!.click());
    expect(onVoiceStop).toHaveBeenCalledTimes(1);
  });

  it.each([100, 350])("handles a %i ms pointer press without cancelling on capture release", elapsed => {
    const onVoiceStart = vi.fn();
    const onVoiceStop = vi.fn();
    const onVoiceCancel = vi.fn();
    render({ voiceAvailable: true, onVoiceStart, onVoiceStop, onVoiceCancel });
    const mic = container.querySelector<HTMLButtonElement>('button[aria-label="Hold to dictate"]')!;
    mic.setPointerCapture = vi.fn();
    mic.hasPointerCapture = () => true;
    mic.releasePointerCapture = () => { mic.dispatchEvent(new Event("lostpointercapture", { bubbles: true })); };
    const now = vi.spyOn(Date, "now").mockReturnValue(0);
    act(() => mic.dispatchEvent(new MouseEvent("pointerdown", { button: 0, bubbles: true })));
    now.mockReturnValue(elapsed);
    act(() => mic.dispatchEvent(new MouseEvent("pointerup", { button: 0, bubbles: true })));
    now.mockRestore();
    expect(onVoiceStart).toHaveBeenCalledTimes(1);
    expect(onVoiceStop).toHaveBeenCalledTimes(elapsed >= 300 ? 1 : 0);
    expect(onVoiceCancel).not.toHaveBeenCalled();
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

describe("browser context attachments", () => {
  it("shows removable untrusted context while preserving the user's prompt", () => {
    const onRemoveBrowserSelection = vi.fn();
    const onChange = vi.fn();
    render({ value: "Make this blue", onChange, onRemoveBrowserSelection, browserSelections: [{
      id: "selected-1", sessionId: "task-1", tabId: "tab-1", navigationId: 2,
      url: "http://localhost:3000/", title: "Preview", selector: "button", snippet: "<button>Save</button>",
      bounds: { x: 0, y: 0, width: 100, height: 40 }, annotations: [],
    }] });
    expect(container.textContent).toContain("Page element");
    act(() => container.querySelector<HTMLButtonElement>('button[aria-label="Remove browser selection"]')!.click());
    expect(onRemoveBrowserSelection).toHaveBeenCalledWith("selected-1");
    expect(onChange).not.toHaveBeenCalled();
    expect(textarea().value).toBe("Make this blue");
  });
});
