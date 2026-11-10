// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CI_TOAST_TTL_MS, GithubToasts, type CiToast } from "./GithubToasts";

let root: Root | undefined;
let host: HTMLDivElement | undefined;

const failed: CiToast = { key: "w#341:2/5", payload: { workspaceId: "w", number: 341, headBranch: "feat/cursor-sidebar-dev", title: "Sidebar dev", failed: 2, total: 5 } };
const passed: CiToast = { key: "w#7:0/3", payload: { workspaceId: "w", number: 7, headBranch: "feat/green", title: "Green run", failed: 0, total: 3 } };

async function mount(ui: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => { root?.render(ui); });
}

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks(); vi.useRealTimers();
});

describe("GithubToasts", () => {
  it("renders the CI verdicts, opens on click, and dismisses on the X", async () => {
    const onOpen = vi.fn();
    const onDismiss = vi.fn();
    await mount(<GithubToasts toasts={[failed, passed]} onOpen={onOpen} onDismiss={onDismiss} onDismissHint={() => undefined} />);
    expect(host!.textContent).toContain("CI failed on feat/cursor-sidebar-dev — 2 checks");
    expect(host!.textContent).toContain("#341 Sidebar dev");
    expect(host!.textContent).toContain("CI passed on feat/green");

    (host!.querySelector('button[aria-label*="open the pull request"]') as HTMLButtonElement).click();
    expect(onOpen).toHaveBeenCalledWith(failed);
    (host!.querySelector('button[aria-label="Dismiss notification"]') as HTMLButtonElement).click();
    expect(onDismiss).toHaveBeenCalledWith(failed.key);
  });

  it("auto-dismisses a card after its TTL", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await mount(<GithubToasts toasts={[failed]} onOpen={() => undefined} onDismiss={onDismiss} onDismissHint={() => undefined} />);
    await act(async () => { vi.advanceTimersByTime(CI_TOAST_TTL_MS + 1); });
    expect(onDismiss).toHaveBeenCalledWith(failed.key);
  });

  it("shows the jump fallback hint and lets it be dismissed", async () => {
    const onDismissHint = vi.fn();
    await mount(<GithubToasts toasts={[]} hint="feat/x isn’t checked out here — showing the file on main." onOpen={() => undefined} onDismiss={() => undefined} onDismissHint={onDismissHint} />);
    expect(host!.textContent).toContain("isn’t checked out here");
    (host!.querySelector('button[aria-label="Dismiss hint"]') as HTMLButtonElement).click();
    expect(onDismissHint).toHaveBeenCalled();
  });

  it("expires on time while the parent re-renders with a fresh onDismiss", async () => {
    // App recreates its handlers on every render, and its git poll renders
    // every few seconds; that must not keep a CI verdict on screen forever.
    vi.useFakeTimers();
    const dismissed: string[] = [];
    const render = () => root?.render(<GithubToasts toasts={[failed]} onOpen={() => undefined} onDismiss={key => dismissed.push(key)} onDismissHint={() => undefined} />);
    await mount(<GithubToasts toasts={[failed]} onOpen={() => undefined} onDismiss={key => dismissed.push(key)} onDismissHint={() => undefined} />);
    for (let elapsed = 0; elapsed <= CI_TOAST_TTL_MS; elapsed += 5_000) {
      await act(async () => { vi.advanceTimersByTime(5_000); render(); });
    }
    expect(dismissed).toContain(failed.key);
  });

  it("shows the newest three verdicts and counts the rest", async () => {
    const many: CiToast[] = Array.from({ length: 5 }, (_, index) => ({ ...passed, key: `w#${index}:0/3`, payload: { ...passed.payload, number: index } }));
    await mount(<GithubToasts toasts={many} onOpen={() => undefined} onDismiss={() => undefined} onDismissHint={() => undefined} />);
    expect(host!.querySelectorAll('button[aria-label="Dismiss notification"]')).toHaveLength(3);
    expect(host!.textContent).toContain("+2 more waiting");
  });

  it("renders nothing when there is nothing to say", async () => {
    await mount(<GithubToasts toasts={[]} onOpen={() => undefined} onDismiss={() => undefined} onDismissHint={() => undefined} />);
    expect(host!.textContent).toBe("");
  });
});
