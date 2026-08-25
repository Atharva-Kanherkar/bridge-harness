// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { RightRailPreview } from "./RightRailPreview";

let container: HTMLDivElement;
let root: Root;

function click(label: string) {
  const button = [...container.querySelectorAll("button")].find(node =>
    (node.getAttribute("aria-label") ?? node.textContent ?? "").includes(label),
  );
  if (!button) throw new Error(`No button matching ${label}`);
  act(() => button.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => { root.render(<RightRailPreview />); });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("RightRailPreview", () => {
  it("puts the rail after the canvas, with no Work/Code switch and only Repositories", () => {
    const canvas = container.querySelector('[aria-label="Canvas"]');
    const rail = container.querySelector('[aria-label="Sidebar"]');
    expect(canvas && rail && (canvas.compareDocumentPosition(rail) & Node.DOCUMENT_POSITION_FOLLOWING)).toBeTruthy();
    expect(rail?.getAttribute("data-preview-rail")).toBe("right");
    expect(container.textContent).toContain("Repositories");
    expect(container.querySelector('[aria-label="Chat scope"]')).toBeNull();
    expect(container.textContent).not.toContain("Needs you");
    expect([...container.querySelectorAll("span")].some(node => node.textContent === "Chats")).toBe(false);
  });

  it("opens a new chat in the current repo instead of asking where it should run", () => {
    click("New Chat");
    expect(container.textContent).not.toContain("Where should it run?");
    expect(container.textContent).toContain("bridge-harness");
    expect(container.textContent).toContain("feat/cursor-sidebar-dev");
    expect(container.textContent).toContain("This Mac");
    expect(container.textContent).toContain("On branch");
    expect(container.querySelector('textarea[aria-label="Message"]')).toBeTruthy();
  });

  it("opens Automations only, not Agents", () => {
    click("Automations");
    expect(container.textContent).toContain("Automations");
    expect(container.textContent).toContain("Scheduled jobs only");
    expect(container.querySelector('[aria-label="Marketplace sections"]')).toBeNull();
    expect(container.querySelector("h1")?.textContent).toBe("Automations");
  });

  it("lets the host chip pick Cloud or SSH and drafts those icons", () => {
    click("This Mac");
    const hostMenu = container.querySelector('[role="menu"][aria-label="Agent host"]');
    expect(hostMenu?.textContent).toContain("SSH");
    expect(hostMenu?.textContent).toContain("Cloud");
    const ssh = [...container.querySelectorAll('[role="menuitem"]')].find(node => node.textContent?.includes("SSH"));
    act(() => ssh?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect([...container.querySelectorAll("button")].some(node => (node.getAttribute("aria-expanded") === "false" || node.getAttribute("aria-expanded") === "true") && (node.textContent ?? "").includes("SSH"))).toBe(true);
  });

  it("makes the window a flush rectangle in fullscreen", () => {
    click("Fullscreen");
    expect(container.querySelector("[data-preview-frame='fullscreen']")?.className).toContain("rounded-none");
    click("Windowed");
    expect(container.querySelector("[data-preview-frame='windowed']")?.className).toContain("rounded-window");
  });
});
