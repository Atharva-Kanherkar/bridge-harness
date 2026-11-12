// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ToastCard, ToastOverflow } from "./ToastCard";

let root: Root | undefined;
let host: HTMLDivElement | undefined;

async function render(ui: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  if (!root) {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  }
  await act(async () => { root?.render(ui); });
}

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove();
  root = undefined;
  host = undefined;
  vi.useRealTimers();
});

const card = (props: Partial<React.ComponentProps<typeof ToastCard>> = {}) => <ToastCard
  toastKey="k"
  durationMs={5_000}
  onDismiss={() => undefined}
  icon={<span />}
  body={<span>Turn completed</span>}
  {...props}
/>;

describe("ToastCard", () => {
  it("expires after its duration, and never without one", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await render(card({ onDismiss }));
    await act(async () => { vi.advanceTimersByTime(4_999); });
    expect(onDismiss).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(2); });
    expect(onDismiss).toHaveBeenCalledWith("k");

    const sticky = vi.fn();
    await render(card({ toastKey: "sticky", durationMs: null, onDismiss: sticky }));
    await act(async () => { vi.advanceTimersByTime(600_000); });
    expect(sticky).not.toHaveBeenCalled();
  });

  it("keeps its timer through a re-render with a new onDismiss, and restarts on a new occurrence", async () => {
    vi.useFakeTimers();
    const dismissed: string[] = [];
    await render(card({ firedAt: 1, onDismiss: key => dismissed.push(key) }));
    await act(async () => { vi.advanceTimersByTime(4_000); });
    await render(card({ firedAt: 1, onDismiss: key => dismissed.push(`${key}!`) }));
    await act(async () => { vi.advanceTimersByTime(1_001); });
    expect(dismissed, "the original deadline, through the latest handler").toEqual(["k!"]);

    dismissed.length = 0;
    await render(card({ firedAt: 2, onDismiss: key => dismissed.push(key) }));
    await act(async () => { vi.advanceTimersByTime(4_999); });
    expect(dismissed, "a fresh occurrence gets its full time").toEqual([]);
    await act(async () => { vi.advanceTimersByTime(2); });
    expect(dismissed).toEqual(["k"]);
  });

  it("opens from the body, dismisses from the X, and is plain text without onOpen", async () => {
    const onOpen = vi.fn();
    const onDismiss = vi.fn();
    await render(card({ onOpen, onDismiss, openLabel: "Turn completed — open the chat" }));
    (host!.querySelector('button[aria-label="Turn completed — open the chat"]') as HTMLButtonElement).click();
    (host!.querySelector('button[aria-label="Dismiss notification"]') as HTMLButtonElement).click();
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledWith("k");

    await render(card());
    expect(host!.querySelectorAll("button")).toHaveLength(1);
    expect(host!.textContent).toContain("Turn completed");
  });

  it("counts what waits behind the shown toasts", async () => {
    await render(<ToastOverflow count={2} />);
    expect(host!.textContent).toBe("+2 more waiting");
    await render(<ToastOverflow count={0} />);
    expect(host!.textContent).toBe("");
  });
});
