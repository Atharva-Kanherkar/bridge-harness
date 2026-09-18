// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkBriefingOptions, WorkSettings, WorkSettingsSnapshot } from "../protocol/generated/protocol";

// The api is mocked wholesale: this file tests the form's behaviour, and the
// validation the form pre-empts is tested where it is enforced, in Rust.
const readWorkSettings = vi.fn<() => Promise<WorkSettingsSnapshot>>();
const workBriefingOptions = vi.fn<() => Promise<WorkBriefingOptions>>();
const writeWorkSettings = vi.fn<(settings: WorkSettings) => Promise<WorkSettingsSnapshot>>();
vi.mock("../api", () => ({
  bridgeApi: {
    readWorkSettings: () => readWorkSettings(),
    workBriefingOptions: () => workBriefingOptions(),
    writeWorkSettings: (settings: WorkSettings) => writeWorkSettings(settings),
  },
}));

import { WorkSettingsSection } from "./WorkSettingsSection";

function settings(overrides: Partial<WorkSettings> = {}): WorkSettings {
  return {
    briefing: { harness: "claude", model: "haiku", effort: null },
    enabledConnectorInstances: [],
    refreshOnFocus: false,
    refreshIntervalMinutes: null,
    cooldownMinutes: 15,
    limits: { maxWallSeconds: 600, maxTurns: 12, maxToolCalls: 24, maxOutputTokens: null, costCeilingMicrousd: null },
    ...overrides,
  };
}

function options(): WorkBriefingOptions {
  return {
    harnesses: [
      {
        id: "claude",
        label: "Claude Code",
        available: true,
        supported: true,
        reason: null,
        defaultModel: "haiku",
        models: [{ id: "haiku", label: "Claude Haiku", tier: "fast", defaultForBriefing: true }],
        connectors: [
          { id: "claude.ai Slack", family: "slack", connected: true },
          { id: "claude.ai GitHub", family: "github", connected: true },
          { id: "claude.ai Gmail", family: "gmail", connected: false },
        ],
      },
      {
        id: "codex",
        label: "Codex",
        available: true,
        supported: false,
        reason: "no per-tool authority",
        defaultModel: null,
        models: [],
        connectors: [],
      },
    ],
  };
}

let host: HTMLDivElement;
let root: Root;

async function render(onOpenBoard?: () => void) {
  await act(async () => {
    root.render(<WorkSettingsSection onError={() => undefined} onOpenBoard={onOpenBoard} />);
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  readWorkSettings.mockResolvedValue({ configured: true, settings: settings() });
  workBriefingOptions.mockResolvedValue(options());
  writeWorkSettings.mockImplementation(async stored => ({ configured: true, settings: stored }));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.clearAllMocks();
});

const text = () => host.textContent ?? "";
/** The switch belonging to a row, found by the row's own label. */
const switchFor = (label: string) =>
  host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${label}"]`);
const click = (element: HTMLElement | null | undefined) => act(async () => element?.click());

describe("Work briefing", () => {
  // Every control on this page persists on change, so the page carries no save
  // bar and no native input at all.
  it("uses switches and listboxes, never a native checkbox or select", async () => {
    await render();
    expect(host.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    expect(host.querySelectorAll("select")).toHaveLength(0);
    expect(text()).not.toContain("Save Work settings");
  });

  it("offers only the harness's own connectors, defaulting to everything", async () => {
    await render();
    expect(text()).toContain("What it reads");
    expect(switchFor("Everything connected")?.getAttribute("aria-checked")).toBe("true");
    // The individual list is folded away while everything is selected.
    expect(text()).not.toContain("claude.ai Slack");
  });

  it("narrowing starts from every signed-in tool and persists the picked ids", async () => {
    await render();
    await click(switchFor("Everything connected"));
    // All three appear; the signed-in two start on, Gmail cannot be picked.
    expect(text()).toContain("claude.ai Slack");
    expect(switchFor("claude.ai Gmail")?.disabled).toBe(true);
    expect(text()).toContain("Needs sign-in in Claude Code");
    expect(writeWorkSettings.mock.calls.at(-1)![0].enabledConnectorInstances)
      .toEqual(["claude.ai Slack", "claude.ai GitHub"]);

    await click(switchFor("claude.ai GitHub"));
    expect(writeWorkSettings.mock.calls.at(-1)![0].enabledConnectorInstances).toEqual(["claude.ai Slack"]);
  });

  it("refuses to persist a narrowing with nothing in it", async () => {
    await render();
    await click(switchFor("Everything connected"));
    await click(switchFor("claude.ai Slack"));
    const before = writeWorkSettings.mock.calls.length;
    await click(switchFor("claude.ai GitHub"));
    expect(text()).toContain("Pick at least one tool");
    // The empty narrowing is held locally rather than sent and bounced.
    expect(writeWorkSettings.mock.calls).toHaveLength(before);
  });

  it("switching back to everything stores the empty list, not the full one", async () => {
    readWorkSettings.mockResolvedValue({
      configured: true,
      settings: settings({ enabledConnectorInstances: ["claude.ai Slack"] }),
    });
    await render();
    // A stored narrowing renders as narrowed with the stored id on.
    expect(switchFor("Everything connected")?.getAttribute("aria-checked")).toBe("false");
    expect(switchFor("claude.ai Slack")?.getAttribute("aria-checked")).toBe("true");
    expect(switchFor("claude.ai GitHub")?.getAttribute("aria-checked")).toBe("false");

    await click(switchFor("Everything connected"));
    expect(writeWorkSettings.mock.calls.at(-1)![0].enabledConnectorInstances).toEqual([]);
  });

  it("shows no connector group for a refused harness or when briefing is off", async () => {
    readWorkSettings.mockResolvedValue({ configured: true, settings: settings({ briefing: null }) });
    await render();
    expect(text()).not.toContain("What it reads");
    expect(text()).toContain("Codex");
    expect(text()).toContain("no per-tool authority");
    // Off is still a reachable, stored choice, so the switch is still there.
    expect(switchFor("Integration briefing")?.getAttribute("aria-checked")).toBe("false");
  });

  it("persists the read-mentions toggle and defaults it off", async () => {
    // The setting exists to make the pipe testable: an already-read mention is
    // reproducible on demand, where genuinely new activity is not.
    await render();
    expect(switchFor("Include mentions you have already read")?.getAttribute("aria-checked")).toBe("false");

    await click(switchFor("Include mentions you have already read"));

    expect(writeWorkSettings.mock.calls.at(-1)![0].includeReadMentions).toBe(true);
  });

  it("keeps cadence and refresh on focus, and persists each on change", async () => {
    await render();
    await click(switchFor("Refresh on focus"));
    expect(writeWorkSettings.mock.calls.at(-1)![0].refreshOnFocus).toBe(true);

    const cadence = host.querySelector<HTMLButtonElement>('button[aria-label="Cadence"]')!;
    await click(cadence);
    const hourly = [...document.querySelectorAll<HTMLElement>('[role="listbox"][aria-label="Cadence"] [role="option"]')]
      .find(option => option.textContent?.includes("Every hour"))!;
    await act(async () => {
      hourly.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      hourly.click();
    });
    expect(writeWorkSettings.mock.calls.at(-1)![0].refreshIntervalMinutes).toBe(60);
  });
});

 it("opens the integration board from Work settings", async () => {
   const open = vi.fn(); await render(open);
   await click(Array.from(host.querySelectorAll("button")).find(button => button.textContent === "Open Work"));
   expect(open).toHaveBeenCalledOnce();
   expect(text()).toContain("past 24 hours");
 });
