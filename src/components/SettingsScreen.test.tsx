// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsScreen } from "./SettingsScreen";
import { ALL_PAGES, PAGES, type PageId } from "./settings/sections";
import { SETTINGS_ROWS } from "./settings/settingsSearch";

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

/** React tracks an input's value, so a direct assignment is invisible to it. */
function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find(candidate =>
    candidate.getAttribute("aria-label") === label || (candidate.getAttribute("aria-label") !== "Settings section" && candidate.textContent?.includes(label)));
  if (!match) throw new Error(`Button ${label} was not rendered`);
  return match;
}

const props = {
  adapters: [],
  onModelSetupChange: () => undefined,
  onSuggestionSettingsChange: () => undefined,
  onError: () => undefined,
};

describe("SettingsScreen", () => {
  it("makes every page a rail destination", () => {
    const html = renderToStaticMarkup(<SettingsScreen {...props} />);
    for (const page of ALL_PAGES) expect(html).toContain(`>${PAGES[page].label.replace("&", "&amp;")}<`);
    for (const label of ["Presets", "Harnesses", "Clones", "Runtimes"]) expect(html).not.toContain(`>${label}<`);
  });

  // The header bar and the shield note are both gone: one described a Settings
  // screen that no longer exists, the other stated a prompt fact on a surface
  // that is not only prompts.
  it("drops the old header subtitle and the rail's shield note", () => {
    const html = renderToStaticMarkup(<SettingsScreen {...props} />);
    expect(html).not.toContain("Providers, models, prompts, and agent presets.");
    expect(html).not.toContain("Prompts change behavior, never permissions");
  });

  it("puts Reset all in the rail footer, behind a confirmation that names what it deletes", async () => {
    const html = renderToStaticMarkup(<SettingsScreen {...props} />);
    expect(html).toContain("Reset agents, models and instructions");
    expect(html).not.toContain("Reset all settings");
    expect(html).not.toContain("removes your agents");
  });

  describe("mounted", () => {
    let container: HTMLDivElement;
    let root: Root;

    beforeEach(() => {
      (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
      delete document.documentElement.dataset.skin;
      container = document.createElement("div");
      document.body.append(container);
      root = createRoot(container);
    });

    afterEach(async () => {
      await act(async () => root.unmount());
      container.remove();
      delete document.documentElement.dataset.skin;
    });

    const render = async (extra: Record<string, unknown> = {}) => {
      await act(async () => {
        root.render(<SettingsScreen {...props} {...extra} />);
        await flush();
      });
    };

    const open = async (label: string) => {
      await act(async () => {
        button(container, label).click();
        await flush();
      });
    };

    const waitFor = async (check: () => boolean, label: string) => {
      for (let i = 0; i < 60 && !check(); i++) await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
      expect(check(), label).toBe(true);
    };
    const rail = () => container.querySelector("nav[aria-label='Settings']") as HTMLElement;
    const current = () => rail().querySelector('[aria-current="page"]')?.textContent?.trim();
    const title = () => container.querySelector("h2")?.textContent;

    it("opens retired section ids on the page that replaced them", async () => {
      const cases: [string, PageId][] = [["appearance", "general"], ["storage", "data"], ["harnesses", "codingAgents"], ["history", "import"]];
      for (const [legacy, page] of cases) {
        await render({ initialSection: legacy });
        await waitFor(() => title() === PAGES[page].label, `${legacy} title`);
        expect(current(), legacy).toBe(PAGES[page].label);
      }
    });

    it("titles every page with its rail label", async () => {
      for (const page of ALL_PAGES) {
        await render({ initialSection: page });
        await waitFor(() => title() === PAGES[page].label, `${page}: ${title()}`);
        expect(current()).toBe(PAGES[page].label);
      }
    });

    // The search index is only as good as the rows it points at. Every fixed
    // row id has to be drawn by its page, so the index cannot go stale.
    it("renders every search index row on its page", async () => {
      for (const page of ALL_PAGES) {
        const ids = SETTINGS_ROWS.filter(row => row.page === page).map(row => row.id);
        if (ids.length === 0) continue;
        await render({ initialSection: page });
        const missing = () => ids.filter(id => !container.querySelector(`[data-settings-row="${id}"]`));
        await waitFor(() => missing().length === 0, `${page} is missing ${missing().join(", ")}`);
      }
    });

    it("has no hub rows and no Back to links", async () => {
      for (const page of ["general", "codingAgents", "permissions", "import"] as PageId[]) {
        await render({ initialSection: page });
        expect(container.textContent, page).not.toContain("Back to");
      }
      await render({ initialSection: "general" });
      expect(container.textContent).not.toContain("Active-turn behavior");
    });

    it("reveals a search hit inside an Advanced fold: opens it, scrolls to it, and highlights it briefly", async () => {
      const scroll = vi.fn();
      Element.prototype.scrollIntoView = scroll;
      await render({ initialSection: "permissions" });
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => { type(search, "mission control"); await flush(); });
      const hit = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Search results"] button')].find(node => node.textContent?.includes("Show worker chats"))!;
      await act(async () => { hit.click(); await flush(); });
      const row = () => container.querySelector<HTMLElement>('[data-settings-row="mission-control"]');
      await waitFor(() => row()?.hasAttribute("data-search-highlight") === true, "highlighted");
      expect(current()).toBe("General");
      expect(row()!.closest("details")!.open).toBe(true);
      expect(scroll).toHaveBeenCalled();
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 1700)); });
      expect(row()!.hasAttribute("data-search-highlight")).toBe(false);
    });

    it("opens a harness hit on that harness's detail page", async () => {
      Element.prototype.scrollIntoView = vi.fn();
      await render({ initialSection: "general" });
      await waitFor(() => !!container.querySelector('input[type="search"]'), "search");
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => { type(search, "claude code"); await flush(); });
      const hit = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Search results"] button')].find(node => node.textContent?.startsWith("Claude Code"))!;
      await act(async () => { hit.click(); await flush(); });
      await waitFor(() => title() === "Claude Code", "harness detail");
      expect(container.querySelector('[aria-label="Breadcrumb"]')?.textContent).toContain("Coding agents");
    });

    it("opens an agent hit on that agent's detail page", async () => {
      Element.prototype.scrollIntoView = vi.fn();
      await render({ initialSection: "general" });
      await waitFor(() => !!container.querySelector('input[type="search"]'), "search");
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => { type(search, "bridge orchestrator"); await flush(); });
      const hit = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Search results"] button')].find(node => node.textContent?.startsWith("Bridge orchestrator"))!;
      await act(async () => { hit.click(); await flush(); });
      await waitFor(() => title() === "Bridge orchestrator", "agent detail");
      expect(current()).toBe("Your agents");
    });

    it("stamps the chosen skin on the document from the Appearance tiles", async () => {
      await render({ initialSection: "appearance" });
      expect(container.textContent).toContain("Window background");

      await act(async () => { button(container, "Vibrancy").click(); await flush(); });
      expect(document.documentElement.dataset.skin).toBe("vibrancy");

      await act(async () => { button(container, "Solid").click(); await flush(); });
      expect(document.documentElement.dataset.skin).toBe("graphite");
    });

    it("persists the thinking control style from the Appearance tiles", async () => {
      const store = new Map<string, string>();
      const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
      Object.defineProperty(globalThis, "localStorage", {
        configurable: true,
        value: {
          getItem: (key: string) => store.get(key) ?? null,
          setItem: (key: string, value: string) => { store.set(key, value); },
          removeItem: (key: string) => { store.delete(key); },
          clear: () => store.clear(),
        },
      });
      try {
        await render({ initialSection: "appearance" });
        expect(container.textContent).toContain("Thinking control");
        const group = container.querySelector('[role="radiogroup"][aria-label="Thinking control"]')!;
        expect(group.querySelector('[role="radio"][aria-checked="true"]')!.textContent).toContain("Slider");

        await act(async () => { button(container, "Sentence").click(); await flush(); });
        expect(store.get("bridge.effortSelector")).toBe("sentence");
        expect(group.querySelector('[role="radio"][aria-checked="true"]')!.textContent).toContain("Sentence");
      } finally {
        if (original) Object.defineProperty(globalThis, "localStorage", original);
        else Reflect.deleteProperty(globalThis, "localStorage");
      }
    });

    it("opens Permissions from initialSection, which is what the bypass badge does", async () => {
      await render({ initialSection: "permissions" });
      expect(container.textContent).toContain("How much Bridge asks before an agent acts.");
      expect(container.querySelector('[role="switch"]')).toBeTruthy();
    });

    it("opens Prompts from initialSection, which is what the usage panel does", async () => {
      await render({ initialSection: "prompts" });
      expect(container.textContent).toContain("bridge_role");
    });

    it("filters rows across every page from the rail search", async () => {
      await render();
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => {
        type(search, "suggestion");
        await flush();
      });
      const results = container.querySelector('[aria-label="Search results"]')!;
      expect(results.textContent).toContain("Typing & search");
      expect(results.textContent).toContain("Suggestion model");
      // A search hit navigates to the page that owns the row.
      await act(async () => {
        results.querySelector<HTMLButtonElement>("button")!.click();
        await flush();
      });
      expect(container.textContent).toContain("Inline suggestions");
    });

    it("follows a changed contextual link while settings is already open", async () => {
      await render({ initialSection: "general" });
      await render({ initialSection: "permissions" });
      expect(container.textContent).toContain("How much Bridge asks before an agent acts.");
      expect(container.querySelector('[aria-current="page"]')?.textContent?.trim()).toBe("Permissions");
    });

    it("says so, rather than showing everything, when nothing matches the search", async () => {
      await render();
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => {
        type(search, "zzzz");
        await flush();
      });
      expect(container.textContent).toContain("No setting matches that.");
    });

    it("opens a harness detail page from a list row, and comes back by breadcrumb", async () => {
      await render({ initialSection: "harnesses" });
      expect(container.textContent).toContain("Installed");
      // Bridge is always on and lives under Defaults rather than in the
      // install/repair groups.
      expect(container.textContent).toContain("Always on");

      await open("Configure Claude Code");
      const crumbs = container.querySelector('[aria-label="Breadcrumb"]')!;
      expect(crumbs.textContent).toContain("Coding agents");
      expect(container.textContent).toContain("Custom instructions");
      expect(container.textContent).toContain("New chats");

      await act(async () => {
        crumbs.querySelector<HTMLButtonElement>("button")!.click();
        await flush();
      });
      expect(container.querySelector('[aria-label="Breadcrumb"]')).toBeNull();
    });

    it("opens a preset detail page from the list, with no third sidebar", async () => {
      await render({ initialSection: "agents" });
      expect(container.textContent).toContain("Bridge orchestrator");
      // The list is rows in the one column; the old build drew its own sidebar
      // here and started content 440px in.
      expect(container.querySelector('[aria-label="Settings"]')).toBeTruthy();

      await open("Edit Bridge orchestrator");
      expect(container.textContent).toContain("Setup");
      expect(container.textContent).toContain("Coding agent & model");
      expect(container.textContent).toContain("Custom instructions");
      expect(container.querySelector('[aria-label="Breadcrumb"]')).toBeTruthy();
    });

    // Row content is covered in settings/ModelsPage.test.tsx, which can supply
    // profiles; the mock setup deliberately ships none.
    // A rail item is named for a page, not for whatever detail was last open on
    // it. The draft still survives, which is what the contract asks for.
    it("returns to the list when the rail item is clicked again, keeping the draft", async () => {
      await render({ initialSection: "agents" });
      await open("Edit Bridge orchestrator");
      expect(container.querySelector('[aria-label="Breadcrumb"]')).toBeTruthy();

      const name = container.querySelector<HTMLInputElement>('input[aria-label="Setup name"]')!;
      type(name, "Renamed orchestrator");
      await act(async () => flush());

      await open("Coding agents");
      await open("Your agents");
      expect(container.querySelector('[aria-label="Breadcrumb"]')).toBeNull();

      await open("Edit Bridge orchestrator");
      expect(container.querySelector<HTMLInputElement>('input[aria-label="Setup name"]')!.value)
        .toBe("Renamed orchestrator");
    });

    it("shows Models with a version pill and no Save button", async () => {
      await render({ initialSection: "models" });
      expect(container.textContent).toContain("Choose models for your chats");
      expect(container.textContent).toContain("Available models");
      expect([...container.querySelectorAll("button")].map(node => node.textContent))
        .not.toContain("Save profiles");
    });

    // The screen-wide invariants. A page that reintroduces a native select or
    // a lucide icon breaks here rather than in review.
    it("renders no native select and no native checkbox on any page", async () => {
      for (const section of ALL_PAGES) {
        await render({ initialSection: section });
        expect(container.querySelectorAll("select"), PAGES[section].label).toHaveLength(0);
        expect(container.querySelectorAll('input[type="checkbox"]'), PAGES[section].label).toHaveLength(0);
        await act(async () => root.unmount());
        container.remove();
        container = document.createElement("div");
        document.body.append(container);
        root = createRoot(container);
      }
    });

    it("keeps the content column the same width on every page", async () => {
      const widths = new Set<string>();
      for (const section of ALL_PAGES) {
        await render({ initialSection: section });
        const column = container.querySelector<HTMLElement>("[data-settings-column]");
        expect(column, PAGES[section].label).toBeTruthy();
        const width = [...column!.classList].find(name => name.startsWith("max-w-"))!;
        // The storage pages carry a disk explorer and a copilot rail side by
        // side; they are the one deliberate exception.
        if (section === "data") expect(width, PAGES[section].label).toBe("max-w-page-wide");
        else widths.add(width);
        await act(async () => root.unmount());
        container.remove();
        container = document.createElement("div");
        document.body.append(container);
        root = createRoot(container);
      }
      expect([...widths]).toEqual(["max-w-page"]);
    });

    it("names what Reset all deletes before it deletes it", async () => {
      await render();
      await open("Reset agents, models and instructions");
      expect(container.textContent).toContain("removes your agents");
      expect(container.textContent).toContain("Keep them");
      await open("Keep them");
      expect(container.textContent).not.toContain("removes your agents");
    });
  });
});
