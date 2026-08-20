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

async function render() {
  await act(async () => {
    root.render(<WorkSettingsSection onError={() => undefined} />);
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
const checkboxes = () => Array.from(host.querySelectorAll<HTMLInputElement>("input[type=checkbox]"));
const checkboxIn = (labelText: string) =>
  Array.from(host.querySelectorAll("label"))
    .find(label => label.textContent?.includes(labelText))
    ?.querySelector<HTMLInputElement>("input[type=checkbox]");
const saveButton = () =>
  Array.from(host.querySelectorAll("button")).find(button => button.textContent?.includes("Save Work settings"))!;

describe("what it reads", () => {
  it("offers only the harness's own connectors, defaulting to everything", async () => {
    await render();
    expect(text()).toContain("What it reads");
    expect(checkboxIn("Everything connected")?.checked).toBe(true);
    // The individual list is folded away while everything is selected.
    expect(text()).not.toContain("claude.ai Slack");
  });

  it("narrowing starts from every signed-in tool and saves the picked ids", async () => {
    await render();
    await act(async () => {
      checkboxIn("Everything connected")?.click();
    });
    // All three appear; the signed-in two start checked, Gmail cannot be picked.
    expect(text()).toContain("claude.ai Slack");
    const gmail = checkboxIn("claude.ai Gmail");
    expect(gmail?.disabled).toBe(true);
    expect(text()).toContain("needs sign-in in Claude Code");

    await act(async () => {
      checkboxIn("claude.ai GitHub")?.click();
    });
    await act(async () => {
      saveButton().click();
    });
    expect(writeWorkSettings).toHaveBeenCalledTimes(1);
    expect(writeWorkSettings.mock.calls[0][0].enabledConnectorInstances).toEqual(["claude.ai Slack"]);
  });

  it("refuses to save a narrowing with nothing in it", async () => {
    await render();
    await act(async () => {
      checkboxIn("Everything connected")?.click();
    });
    await act(async () => {
      checkboxIn("claude.ai Slack")?.click();
    });
    await act(async () => {
      checkboxIn("claude.ai GitHub")?.click();
    });
    expect(text()).toContain("Pick at least one tool");
    expect(saveButton().disabled).toBe(true);
  });

  it("switching back to everything stores the empty list, not the full one", async () => {
    readWorkSettings.mockResolvedValue({
      configured: true,
      settings: settings({ enabledConnectorInstances: ["claude.ai Slack"] }),
    });
    await render();
    // A stored narrowing renders as narrowed with the stored id checked.
    expect(checkboxIn("Everything connected")?.checked).toBe(false);
    expect(checkboxIn("claude.ai Slack")?.checked).toBe(true);
    expect(checkboxIn("claude.ai GitHub")?.checked).toBe(false);

    await act(async () => {
      checkboxIn("Everything connected")?.click();
    });
    await act(async () => {
      saveButton().click();
    });
    expect(writeWorkSettings.mock.calls[0][0].enabledConnectorInstances).toEqual([]);
  });

  it("shows no connector block for a refused harness or when briefing is off", async () => {
    readWorkSettings.mockResolvedValue({ configured: true, settings: settings({ briefing: null }) });
    await render();
    expect(text()).not.toContain("What it reads");
    expect(text()).toContain("Codex");
    expect(text()).toContain("no per-tool authority");
    expect(checkboxes().length).toBeGreaterThan(0);
  });
});
