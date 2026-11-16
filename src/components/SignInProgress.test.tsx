// @vitest-environment jsdom
// Sign-in progress: every provider walks the same visible stages, an account
// adds its folder step, and the pane never sits on a bare spinner while a CLI
// is slow to print.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { TerminalChunk } from "../types";
import { ProviderLoginPane } from "./ProviderLoginPane";
import { activeStepIndex, signInNarration, signInSteps, type SignInStage } from "./SignInProgress";

const STAGES: SignInStage[] = ["preparing", "starting", "browser", "checking", "done"];

describe("sign-in stages", () => {
  it("adds a folder step only for an account", () => {
    expect(signInSteps(false).map(step => step.label)).toEqual(["Start agent", "Sign in", "Confirm"]);
    expect(signInSteps(true).map(step => step.label)).toEqual(["Set up folder", "Start agent", "Sign in", "Confirm"]);
  });

  it("only ever moves forward through the steps, for both kinds of sign-in", () => {
    for (const account of [false, true]) {
      const steps = signInSteps(account);
      const indexes = STAGES.map(stage => activeStepIndex(steps, stage));
      expect([...indexes].sort((a, b) => a - b)).toEqual(indexes);
      expect(indexes.at(-1)).toBe(steps.length);
      expect(indexes[0]).toBe(0);
    }
  });

  it("narrates every stage, and says so when the agent is slow", () => {
    for (const stage of STAGES) for (const account of [false, true]) {
      expect(signInNarration(stage, "Codex", account, false)).toBeTruthy();
    }
    expect(signInNarration("preparing", "Codex", true, false)).toContain("folder");
    expect(signInNarration("starting", "Codex", true, true)).toContain("taking a moment");
  });
});

let container: HTMLDivElement;
let root: Root;
let emit: (chunk: TerminalChunk) => void = () => undefined;
const flush = async () => { await act(async () => {}); };
const stage = () => container.querySelector("[data-stage]")?.getAttribute("data-stage");
const narration = () => container.querySelector('[data-testid="signin-narration"]')?.textContent;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.spyOn(bridgeApi, "onTerminal").mockImplementation(async handler => { emit = handler; return () => undefined; });
  vi.spyOn(bridgeApi, "onTerminalExited").mockResolvedValue(() => undefined);
  vi.spyOn(bridgeApi, "cancelProviderLogin").mockResolvedValue();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const URLS = {
  claude: "https://claude.ai/oauth/authorize?code=true&state=x",
  codex: "https://auth.openai.com/oauth/authorize?state=x",
} as const;

describe.each([["claude", "Claude Code"], ["codex", "Codex"]] as const)("%s account sign-in pane", (provider, label) => {
  it("walks folder → start → browser, with a placeholder where the link lands", async () => {
    let release: () => void = () => undefined;
    vi.spyOn(bridgeApi, "startProviderLogin").mockImplementation(() => new Promise(resolve => {
      release = () => resolve({ workspaceId: "provider-login", terminalId: `${provider}:account:work` });
    }));
    act(() => root.render(<ProviderLoginPane provider={provider} label={label} account="work" onClose={() => undefined} />));
    await flush();
    expect(stage()).toBe("preparing");
    expect(narration()).toContain("folder");
    expect(container.querySelector('[aria-current="step"]')?.textContent).toBe("Set up folder");
    expect(container.querySelector(".skeleton-shimmer")).not.toBeNull();
    expect(container.innerHTML).not.toContain("animate-spin");

    await act(async () => release());
    expect(stage()).toBe("starting");
    expect(container.querySelector('[aria-current="step"]')?.textContent).toBe("Start agent");

    // Output for another account's terminal never moves this pane on.
    act(() => emit({ sessionId: "provider-login", terminalId: `${provider}:account:other`, data: URLS[provider] } as TerminalChunk));
    expect(stage()).toBe("starting");

    act(() => emit({ sessionId: "provider-login", terminalId: `${provider}:account:work`, data: `Open ${URLS[provider]}\n` } as TerminalChunk));
    expect(stage()).toBe("browser");
    expect(container.querySelector('[aria-current="step"]')?.textContent).toBe("Sign in");
    expect(container.querySelector<HTMLAnchorElement>("a[href]")?.href).toBe(URLS[provider]);
    expect(container.querySelector(".skeleton-shimmer")).toBeNull();
  });

  it("tells the person when the agent is slow to start", async () => {
    vi.useFakeTimers();
    vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: `${provider}:account:work` });
    act(() => root.render(<ProviderLoginPane provider={provider} label={label} account="work" onClose={() => undefined} />));
    await flush();
    await flush();
    expect(narration()).not.toContain("taking a moment");
    act(() => { vi.advanceTimersByTime(13_000); });
    expect(narration()).toContain("taking a moment");
  });
});

it("shows the same progress for the user's own login, without the folder step", async () => {
  vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
  act(() => root.render(<ProviderLoginPane provider="codex" label="Codex" bare onClose={() => undefined} />));
  await flush();
  await flush();
  expect(container.textContent).not.toContain("Set up folder");
  expect(container.querySelector('[aria-current="step"]')?.textContent).toBe("Start agent");
});
