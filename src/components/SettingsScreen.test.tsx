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

    it("mounts PromptStudio when the Prompts nav item is selected", async () => {
      await act(async () => {
        root.render(<SettingsScreen adapters={[]} onModelSetupChange={() => undefined} onSuggestionSettingsChange={() => undefined} onError={() => undefined} />);
        await flush();
      });
      await act(async () => {
        button(container, "Prompts").click();
        await flush();
      });
      expect(container.textContent).toContain("Orchestrator");
      expect(container.textContent).toContain("Direct session");
      expect(container.textContent).toContain("bridge_role");
    });
  });
});
