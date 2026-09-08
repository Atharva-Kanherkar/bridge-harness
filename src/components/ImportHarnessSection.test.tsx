// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImportHarnessSection } from "./ImportHarnessSection";

const openMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

function button(container: HTMLElement, label: string) {
  const result = [...container.querySelectorAll("button")].find(item => item.textContent?.includes(label));
  if (!result) throw new Error(`Button ${label} was not rendered`);
  return result as HTMLButtonElement;
}

/** The wizard's selection controls are switches now, not native checkboxes. */
function switches(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
}

describe("ImportHarnessSection", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    openMock.mockReset();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("states the local-only, read-only, secret, and backup boundaries before discovery", () => {
    const html = renderToStaticMarkup(<ImportHarnessSection onError={() => undefined} />);
    expect(html).toContain("Local only");
    expect(html).toContain("nothing is stored until the final confirmation");
    expect(html).toContain("Bridge reads only what you choose");
    expect(html).toContain("Secrets excluded");
    expect(html).toContain("Not an encrypted backup");
  });

  it("adopts the settings chrome without a native checkbox or select", async () => {
    openMock.mockResolvedValue("/mock/.claude");
    await act(async () => {
      root.render(<ImportHarnessSection onError={error => { throw new Error(error); }} />);
      await flush();
    });
    // One centered column, like every other settings page.
    expect(container.querySelector("[data-settings-column]")?.classList.contains("max-w-page")).toBe(true);
    await act(async () => { button(container, "Choose folder").click(); await flush(); });
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    expect(container.querySelectorAll("select")).toHaveLength(0);
  });

  it("requires conservative selection and a final confirmation before commit", async () => {
    openMock.mockResolvedValue("/mock/.claude");
    await act(async () => {
      root.render(<ImportHarnessSection onError={error => { throw new Error(error); }} />);
      await flush();
    });

    await act(async () => {
      button(container, "Choose folder").click();
      await flush();
    });
    expect(container.textContent).toContain("Discovered");
    expect(container.textContent).toContain("Nothing is selected by default");
    expect(button(container, "Preview selected").disabled).toBe(true);

    // Two acts: the switch has to have re-rendered before Preview stops being
    // disabled, and a disabled button swallows the click.
    await act(async () => { switches(container)[2].click(); await flush(); });
    await act(async () => { button(container, "Preview selected").click(); await flush(); });
    expect(container.textContent).toContain("Candidates");
    expect(container.textContent).toContain("Historical Claude session");
    expect(button(container, "Review exact commit").disabled).toBe(true);

    await act(async () => {
      switches(container)[0].click();
      await flush();
    });
    await act(async () => button(container, "Review exact commit").click());
    expect(container.textContent).toContain("Final confirmation");
    expect(container.textContent).toContain("cannot resume the foreign Claude session");

    await act(async () => {
      button(container, "Import 1 selected").click();
      await flush();
    });
    expect(container.textContent).toContain("Import committed");
    expect(container.textContent).toContain("1 imported");
  });
});
