// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { CloneRequestInbox } from "./CloneRequestInbox";

vi.mock("../api", () => ({ bridgeApi: {
  cloneRequests: vi.fn(), readCloneSettings: vi.fn(), resolveCloneRequest: vi.fn(),
} }));
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.mocked(bridgeApi.cloneRequests).mockReset().mockResolvedValue([]);
  vi.mocked(bridgeApi.readCloneSettings).mockReset().mockResolvedValue({ connected: true, settings: { defaultSignInPath: "sign_in_inside", ttlMinutes: 30 } });
  vi.mocked(bridgeApi.resolveCloneRequest).mockReset().mockResolvedValue({} as never);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); });

it("surfaces background requests without a dock and submits the displayed identity and choices", async () => {
  const request = { sessionId: "other-chat", requestId: "immutable-1", domain: "example.test", extensionPath: "/tmp/extension-under-test" };
  vi.mocked(bridgeApi.cloneRequests).mockResolvedValue([request]);
  const onOpen = vi.fn();
  await act(async () => root.render(<CloneRequestInbox sessionLabels={{ "other-chat": "Other chat" }} onOpen={onOpen} onError={vi.fn()} />));
  expect(host.textContent).toContain("Other chat · browser request");
  expect(host.textContent).toContain(request.extensionPath);
  const lifetime = host.querySelector<HTMLSelectElement>('[aria-label="Browser request lifetime"]')!;
  await act(async () => { lifetime.value = "60"; lifetime.dispatchEvent(new Event("change", { bubbles: true })); });
  const allow = [...host.querySelectorAll("button")].find(button => button.textContent === "Allow")!;
  await act(async () => allow.click());
  expect(bridgeApi.resolveCloneRequest).toHaveBeenCalledWith("other-chat", true, "immutable-1", { defaultSignInPath: "sign_in_inside", ttlMinutes: 60 });
  expect(onOpen).toHaveBeenCalledWith("other-chat");
});

it("keeps simultaneous chats separate and denies the selected request", async () => {
  vi.mocked(bridgeApi.cloneRequests).mockResolvedValue([
    { sessionId: "a", requestId: "a1", domain: "one.test" },
    { sessionId: "b", requestId: "b1", domain: "two.test" },
  ]);
  await act(async () => root.render(<CloneRequestInbox sessionLabels={{}} onOpen={vi.fn()} onError={vi.fn()} />));
  const card = host.querySelector('[aria-label="Browser request for two.test"]')!;
  const deny = [...card.querySelectorAll("button")].find(button => button.textContent === "Deny")!;
  await act(async () => deny.click());
  expect(bridgeApi.resolveCloneRequest).toHaveBeenCalledWith("b", false, "b1", { defaultSignInPath: "sign_in_inside", ttlMinutes: 30 });
  expect(bridgeApi.resolveCloneRequest).toHaveBeenCalledTimes(1);
});

it("continues polling when initially empty and removes a revoked request", async () => {
  await act(async () => root.render(<CloneRequestInbox sessionLabels={{}} onOpen={vi.fn()} onError={vi.fn()} />));
  expect(host.textContent).toBe("");
  vi.mocked(bridgeApi.cloneRequests).mockResolvedValue([{ sessionId: "new", requestId: "new-1", domain: "new.test" }]);
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(host.textContent).toContain("new.test");
  vi.mocked(bridgeApi.cloneRequests).mockResolvedValue([]);
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(host.textContent).toBe("");
});
