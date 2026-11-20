// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { chordLabel } from "../keymap";
import type { Workspace } from "../types";
import { ComposerContextStrip, type ComposerContextStripProps } from "./ComposerContextStrip";
import { ComposerKeyHints, composerKeyHints } from "./ComposerKeyHints";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const render = (node: React.ReactNode) => act(() => { root.render(node); });
const hints = () => container.querySelector<HTMLElement>("[data-composer-keys]")!;

describe("composerKeyHints", () => {
  it("teaches Send and New line while idle", () => {
    expect(composerKeyHints("idle")).toEqual([{ keys: "↵", label: "Send" }, { keys: "⇧↵", label: "New line" }]);
  });

  it("teaches Steer or Queue, and Stop on the interrupt chord, while a turn runs", () => {
    expect(composerKeyHints("steer")).toEqual([{ keys: "↵", label: "Steer" }, { keys: chordLabel("interrupt-turn"), label: "Stop" }]);
    expect(composerKeyHints("queue")).toEqual([{ keys: "↵", label: "Queue" }, { keys: chordLabel("interrupt-turn"), label: "Stop" }]);
  });
});

describe("ComposerKeyHints", () => {
  it("draws each key as a key cap beside its action and switches with the state", () => {
    render(<ComposerKeyHints state="idle" />);
    expect(hints().textContent).toBe("↵Send⇧↵New line");
    expect([...hints().querySelectorAll("kbd")].map(key => key.textContent)).toEqual(["↵", "⇧↵"]);
    render(<ComposerKeyHints state="steer" />);
    expect(hints().dataset.composerKeys).toBe("steer");
    expect([...hints().querySelectorAll("kbd")].map(key => key.textContent)).toEqual(["↵", chordLabel("interrupt-turn")]);
    expect(hints().textContent).toBe(`↵Steer${chordLabel("interrupt-turn")}Stop`);
    render(<ComposerKeyHints state="queue" />);
    expect(hints().textContent).toBe(`↵Queue${chordLabel("interrupt-turn")}Stop`);
  });
});

describe("ComposerContextStrip trailing", () => {
  const workspace = { id: "w", title: "bridge-harness", branch: "main", status: "ready", dirtyFiles: 0, projectId: "p" } as Workspace;
  const strip = (overrides: Partial<ComposerContextStripProps> = {}) => (
    <ComposerContextStrip
      workspaces={[workspace]}
      workspace={workspace}
      worktree={false}
      locked
      branches={[]}
      onSelectWorkspace={() => {}}
      onRequestBranches={() => {}}
      onSelectBranch={() => {}}
      onToggleWorktree={() => {}}
      {...overrides}
    />
  );

  it("puts the hints at the strip's right end, from a medium width up", () => {
    render(strip({ trailing: <ComposerKeyHints state="steer" /> }));
    const row = container.querySelector('[aria-label="Chat context"]')!;
    const slot = row.lastElementChild!;
    expect(slot.contains(hints())).toBe(true);
    expect(slot.className).toContain("ml-auto");
    expect(slot.className).toContain("hidden");
    expect(slot.className).toContain("@xl/composer-context:flex");
  });

  it("leaves the right edge to the agent menu when there is one", () => {
    render(strip({ trailing: <ComposerKeyHints state="idle" />, onSelectAgent: () => {} }));
    const slot = container.querySelector('[aria-label="Chat context"]')!.lastElementChild!;
    expect(slot.contains(hints())).toBe(true);
    expect(slot.className).not.toContain("ml-auto");
  });

  it("draws nothing extra without hints", () => {
    render(strip());
    expect(container.querySelector("[data-composer-keys]")).toBeNull();
  });
});
