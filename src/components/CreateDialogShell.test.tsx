// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CreateDialogShell } from "./CreateDialogShell";
import { WorkspaceCreateDialog } from "./WorkspaceCreateDialog";

// The shell owns the open/closed boundary so every dialog built on it gets an
// exit animation. What matters here is the *lifecycle*: the tree has to survive
// the render that closes it and disappear once the exit lands. Frames themselves
// are Framer's business, and `skipAnimations` collapses them so the assertions
// are about structure rather than timing.

let host: HTMLDivElement;
let root: Root;

function dialog(): HTMLElement | null {
  return host.querySelector<HTMLElement>('[role="dialog"]');
}

/** Let Framer's frame loop run so a finished exit actually unmounts. */
async function settle() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 40));
  });
}

function render(props: Partial<Parameters<typeof CreateDialogShell>[0]> = {}) {
  act(() => {
    root.render(
      <CreateDialogShell
        open
        titleId="shell-title"
        icon={<span data-testid="icon" />}
        title="Start a chat"
        subtitle="Where should it run?"
        closeLabel="Cancel"
        onClose={() => {}}
        {...props}
      >
        <p data-testid="body">body</p>
      </CreateDialogShell>,
    );
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

describe("CreateDialogShell", () => {
  it("renders nothing while closed", () => {
    render({ open: false });
    expect(dialog()).toBeNull();
    expect(host.querySelector('[data-testid="body"]')).toBeNull();
  });

  it("renders the scrim, panel, heading and children when open", () => {
    render();
    const shell = dialog();
    expect(shell).not.toBeNull();
    expect(shell?.getAttribute("aria-modal")).toBe("true");
    expect(shell?.getAttribute("aria-labelledby")).toBe("shell-title");
    expect(host.querySelector("#shell-title")?.textContent).toBe("Start a chat");
    expect(host.querySelector('[data-testid="body"]')).not.toBeNull();
  });

  it("keeps the dialog mounted through the render that closes it, then unmounts it", async () => {
    render();
    expect(dialog()).not.toBeNull();
    render({ open: false });
    // The whole point of the change: closing no longer unmounts same-render.
    expect(dialog()).not.toBeNull();
    // But while it fades it is already logically gone: no clicks land on it,
    // and screen readers stop seeing it.
    expect(dialog()?.className).toContain("pointer-events-none");
    expect(dialog()?.getAttribute("aria-hidden")).toBe("true");
    await settle();
    expect(dialog()).toBeNull();
  });

  it("stays interactive while open", () => {
    render({ dismissOnScrim: true });
    expect(dialog()?.className).not.toContain("pointer-events-none");
    expect(dialog()?.getAttribute("aria-hidden")).toBeNull();
  });

  it("leaves the entrance to Framer rather than the CSS class", () => {
    render();
    const panel = host.querySelector(".u-glass-popover");
    expect(panel).not.toBeNull();
    expect(panel?.className).not.toContain("animate-page-enter");
  });

  it("still closes from the close button and respects closeDisabled", () => {
    const onClose = vi.fn();
    render({ onClose });
    const close = host.querySelector<HTMLButtonElement>('button[aria-label="Cancel"]');
    act(() => close?.click());
    expect(onClose).toHaveBeenCalledTimes(1);

    render({ onClose, closeDisabled: true });
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="Cancel"]')?.disabled).toBe(true);
  });

  it("dismisses on a scrim click and on Escape, but not on a click inside the panel", () => {
    const onClose = vi.fn();
    render({ onClose, dismissOnScrim: true });
    const shell = dialog();
    act(() => {
      shell?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => {
      host.querySelector('[data-testid="body"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => {
      shell?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("dialogs built on the shell", () => {
  it("delegate the open/closed boundary instead of gating themselves", async () => {
    act(() => {
      root.render(<WorkspaceCreateDialog open={false} title="" onTitleChange={() => {}} onClose={() => {}} onSubmit={() => {}} />);
    });
    expect(dialog()).toBeNull();

    act(() => {
      root.render(<WorkspaceCreateDialog open title="" onTitleChange={() => {}} onClose={() => {}} onSubmit={() => {}} />);
    });
    expect(host.querySelector("#workspace-create-title")?.textContent).toBe("New workspace");

    act(() => {
      root.render(<WorkspaceCreateDialog open={false} title="" onTitleChange={() => {}} onClose={() => {}} onSubmit={() => {}} />);
    });
    // Still there: the dialog is animating out, not gone.
    expect(dialog()).not.toBeNull();
    await settle();
    expect(dialog()).toBeNull();
  });
});


describe("creation dialog keyboard behavior", () => {
  it("wraps Tab at both ends and restores the opener on dismissal", async () => {
    const opener = document.createElement("button"); document.body.append(opener); opener.focus();
    render();
    const close = dialog()!.querySelector<HTMLButtonElement>("button")!;
    const forward = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    act(() => { close.focus(); close.dispatchEvent(forward); });
    expect(forward.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(close);
    const backward = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
    act(() => close.dispatchEvent(backward));
    expect(backward.defaultPrevented).toBe(true);
    render({ open: false }); await settle();
    expect(document.activeElement).toBe(opener); opener.remove();
  });

  it("ignores scrim and Escape dismissal while creation is busy", () => {
    const onClose = vi.fn(); render({ closeDisabled: true, dismissOnScrim: true, onClose });
    act(() => { dialog()!.click(); dialog()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(onClose).not.toHaveBeenCalled();
  });
});
