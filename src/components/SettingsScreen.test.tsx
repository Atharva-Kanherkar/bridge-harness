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
  const match = [...container.querySelectorAll("button")].find(candidate => candidate.textContent?.includes(label));
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

      await act(async () => { button(container, "Cursor").click(); await flush(); });
      expect(document.documentElement.dataset.skin).toBe("vibrancy");

      await act(async () => { button(container, "Solid").click(); await flush(); });
      expect(document.documentElement.dataset.skin).toBe("graphite");
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
