// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { SettingsScreen } from "./SettingsScreen";

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find(candidate => candidate.textContent?.includes(label));
  if (!match) throw new Error(`Button ${label} was not rendered`);
  return match;
}

describe("SettingsScreen", () => {
  it("exposes the configuration areas before asynchronous data loads", () => {
    const html = renderToStaticMarkup(<SettingsScreen adapters={[]} onModelSetupChange={() => undefined} onSuggestionSettingsChange={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Settings");
    expect(html).toContain("Agents");
    expect(html).toContain("Harnesses");
    expect(html).toContain("Role models");
    expect(html).toContain("Prompts change behavior, never permissions");
    expect(html).toContain("Permissions");
  });

  describe("Appearance section", () => {
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

    it("stamps the chosen skin on the document from the Theme picker", async () => {
      await act(async () => {
        root.render(<SettingsScreen adapters={[]} onModelSetupChange={() => undefined} onSuggestionSettingsChange={() => undefined} onError={() => undefined} />);
        await flush();
      });
      await act(async () => {
        button(container, "Appearance").click();
        await flush();
      });
      expect(container.textContent).toContain("Theme");
      expect(container.textContent).toContain("Cursor");

      await act(async () => {
        button(container, "Cursor").click();
        await flush();
      });
      expect(document.documentElement.dataset.skin).toBe("vibrancy");

      await act(async () => {
        button(container, "Solid").click();
        await flush();
      });
      expect(document.documentElement.dataset.skin).toBe("graphite");
    });
  });

  describe("Prompt Studio section", () => {
    let container: HTMLDivElement;
    let root: Root;

    beforeEach(() => {
      (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
      container = document.createElement("div");
      document.body.append(container);
      root = createRoot(container);
    });

    afterEach(async () => {
      await act(async () => root.unmount());
      container.remove();
    });

    it("mounts PromptStudio when the Prompt Studio nav item is selected", async () => {
      await act(async () => {
        root.render(<SettingsScreen adapters={[]} onModelSetupChange={() => undefined} onSuggestionSettingsChange={() => undefined} onError={() => undefined} />);
        await flush();
      });
      await act(async () => {
        button(container, "Prompt Studio").click();
        await flush();
      });
      expect(container.textContent).toContain("Orchestrator");
      expect(container.textContent).toContain("Direct session");
      expect(container.textContent).toContain("bridge_role");
    });
  });
});
