// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TRANSIENT_ALERT_TTL_MS, TransientAlert } from "./TransientAlert";

let root: Root | undefined;
let host: HTMLDivElement | undefined;

async function render(message: string, onDismiss: () => void) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  if (!root) {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  }
  await act(async () => {
    root?.render(<TransientAlert title="Something went wrong" message={message} variant="error" onDismiss={onDismiss} />);
  });
}

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove();
  root = undefined;
  host = undefined;
  vi.useRealTimers();
});

describe("TransientAlert", () => {
  it("dismisses after five seconds without restarting on unrelated parent renders", async () => {
    vi.useFakeTimers();
    const firstDismiss = vi.fn();
    const latestDismiss = vi.fn();
    await render("Disk is full", firstDismiss);
    await act(async () => { vi.advanceTimersByTime(TRANSIENT_ALERT_TTL_MS - 1); });
    expect(firstDismiss).not.toHaveBeenCalled();
    await render("Disk is full", latestDismiss);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(firstDismiss).not.toHaveBeenCalled();
    expect(latestDismiss).toHaveBeenCalledTimes(1);
  });

  it("gives a new message its own five seconds", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await render("First warning", onDismiss);
    await act(async () => { vi.advanceTimersByTime(3_000); });
    await render("Second warning", onDismiss);
    await act(async () => { vi.advanceTimersByTime(2_000); });
    expect(onDismiss).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(3_000); });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
