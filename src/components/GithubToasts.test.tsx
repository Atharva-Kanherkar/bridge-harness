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

  it("renders nothing when there is nothing to say", async () => {
    await mount(<GithubToasts toasts={[]} onOpen={() => undefined} onDismiss={() => undefined} onDismissHint={() => undefined} />);
    expect(host!.textContent).toBe("");
  });
});
