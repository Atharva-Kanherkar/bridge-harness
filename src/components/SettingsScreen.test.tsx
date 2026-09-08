// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { SettingsScreen } from "./SettingsScreen";
import { ALL_SECTIONS, SECTION_LABELS } from "./settings/sections";

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
    candidate.getAttribute("aria-label") === label || candidate.textContent?.includes(label));
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
  it("names the four rail groups and all nine pages before data loads", () => {
    const html = renderToStaticMarkup(<SettingsScreen {...props} />);
    for (const group of ["General", "Agents", "Runtimes", "Data"]) expect(html).toContain(group);
    for (const section of ALL_SECTIONS) expect(html).toContain(SECTION_LABELS[section]);
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
    expect(html).toContain("Reset all settings");
    expect(html).not.toContain("removes the agent presets you created");
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

    it("stamps the chosen skin on the document from the Appearance tiles", async () => {
      await render();
      await open("Appearance");
      expect(container.textContent).toContain("Shell");

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
        await render();
        await open("Appearance");
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
      expect(results.textContent).toContain("Composer");
      expect(results.textContent).toContain("Suggestion model");
      // A search hit navigates to the page that owns the row.
      await act(async () => {
        results.querySelector<HTMLButtonElement>("button")!.click();
        await flush();
      });
      expect(container.textContent).toContain("Inline suggestions");
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
      expect(crumbs.textContent).toContain("Harnesses");
      expect(container.textContent).toContain("System prompt");
      expect(container.textContent).toContain("Sessions");

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
      expect(container.querySelectorAll("nav")).toHaveLength(1);

      await open("Edit Bridge orchestrator");
      expect(container.textContent).toContain("Identity");
      expect(container.textContent).toContain("Runtime");
      expect(container.textContent).toContain("System prompt");
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

      const name = container.querySelector<HTMLInputElement>('input[aria-label="Preset name"]')!;
      type(name, "Renamed orchestrator");
      await act(async () => flush());

      await open("Harnesses");
      await open("Presets");
      expect(container.querySelector('[aria-label="Breadcrumb"]')).toBeNull();

      await open("Edit Bridge orchestrator");
      expect(container.querySelector<HTMLInputElement>('input[aria-label="Preset name"]')!.value)
        .toBe("Renamed orchestrator");
    });

    it("shows Models with a version pill and no Save button", async () => {
      await render({ initialSection: "models" });
      expect(container.textContent).toContain("Which model runs each Bridge role");
      expect(container.textContent).toContain("Catalog");
      expect([...container.querySelectorAll("button")].map(node => node.textContent))
        .not.toContain("Save profiles");
    });

    // The screen-wide invariants. A page that reintroduces a native select or
    // a lucide icon breaks here rather than in review.
    it("renders no native select and no native checkbox on any page", async () => {
      for (const section of ALL_SECTIONS) {
        await render({ initialSection: section });
        expect(container.querySelectorAll("select"), SECTION_LABELS[section]).toHaveLength(0);
        expect(container.querySelectorAll('input[type="checkbox"]'), SECTION_LABELS[section]).toHaveLength(0);
        await act(async () => root.unmount());
        container.remove();
        container = document.createElement("div");
        document.body.append(container);
        root = createRoot(container);
      }
    });

    it("keeps the content column the same width on every page", async () => {
      const widths = new Set<string>();
      for (const section of ALL_SECTIONS) {
        await render({ initialSection: section });
        const column = container.querySelector<HTMLElement>("[data-settings-column]");
        expect(column, SECTION_LABELS[section]).toBeTruthy();
        widths.add([...column!.classList].find(name => name.startsWith("max-w-"))!);
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
      await open("Reset all settings");
      expect(container.textContent).toContain("removes the agent presets you created");
      expect(container.textContent).toContain("Keep them");
      await open("Keep them");
      expect(container.textContent).not.toContain("removes the agent presets you created");
    });
  });
});
