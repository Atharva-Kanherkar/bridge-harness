// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { LimitRecoveryView } from "../protocol/generated/protocol";
import { LimitRecoveryBanner } from "./LimitRecoveryBanner";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const pending: LimitRecoveryView = {
  sessionId: "s", mode: "resume", state: "pending", signal: "usage limit", resetAt: new Date(Date.now() + 3_600_000).toISOString(),
  dueAt: new Date(Date.now() + 3_690_000).toISOString(), target: { harness: "claude", model: "opus", effort: null }, switchesModel: false,
  attempts: 1, outcome: null, updatedAt: new Date().toISOString(),
};

const button = (text: string) => [...host.querySelectorAll("button")].find(item => item.textContent === text);

it("shows nothing for a chat without a recovery", async () => {
  vi.spyOn(bridgeApi, "limitRecoveryStatus").mockResolvedValue({ recovery: null });
  await act(async () => { root.render(<LimitRecoveryBanner sessionId="s" sessionStatus="idle" adapters={[]} />); });
  expect(host.innerHTML).toBe("");
});

it("resumes now and cancels through the API, and rereads when the chat's status changes", async () => {
  const status = vi.spyOn(bridgeApi, "limitRecoveryStatus").mockResolvedValue({ recovery: pending });
  await act(async () => { root.render(<LimitRecoveryBanner sessionId="s" sessionStatus="failed" adapters={[]} />); });
  expect(host.querySelector('[data-limit-recovery="pending"]')?.textContent).toContain("Usage limit reached. Bridge continues this chat at");
  const resume = vi.spyOn(bridgeApi, "resumeLimitRecovery").mockResolvedValue({ recovery: { ...pending, state: "done" } });
  await act(async () => { button("Resume now")!.click(); });
  expect(resume).toHaveBeenCalledWith("s");
  expect(host.innerHTML, "a finished recovery hides the banner").toBe("");

  await act(async () => { root.render(<LimitRecoveryBanner sessionId="s" sessionStatus="working" adapters={[]} />); });
  expect(status).toHaveBeenCalledTimes(2);
  const cancel = vi.spyOn(bridgeApi, "cancelLimitRecovery").mockResolvedValue({ recovery: { ...pending, state: "cancelled" } });
  await act(async () => { button("Cancel")!.click(); });
  expect(cancel).toHaveBeenCalledWith("s");
  expect(host.innerHTML).toBe("");
});

it("reports a failed resume and lets a problem be dismissed", async () => {
  vi.spyOn(bridgeApi, "limitRecoveryStatus").mockResolvedValue({ recovery: pending });
  const onError = vi.fn();
  await act(async () => { root.render(<LimitRecoveryBanner sessionId="s" sessionStatus="failed" adapters={[]} onError={onError} />); });
  vi.spyOn(bridgeApi, "resumeLimitRecovery").mockRejectedValue(new Error("Backup harness is unavailable"));
  await act(async () => { button("Resume now")!.click(); });
  expect(onError).toHaveBeenCalledWith("Backup harness is unavailable.");

  vi.spyOn(bridgeApi, "limitRecoveryStatus").mockResolvedValue({ recovery: { ...pending, state: "failed", outcome: "boom" } });
  await act(async () => { root.render(<LimitRecoveryBanner sessionId="s" sessionStatus="idle" adapters={[]} />); });
  expect(host.textContent).toContain("could not continue this chat after the usage limit. Send a message to try again.");
  expect(button("Resume now")).toBeUndefined();
  await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="Dismiss"]')!.click(); });
  expect(host.innerHTML).toBe("");
});
