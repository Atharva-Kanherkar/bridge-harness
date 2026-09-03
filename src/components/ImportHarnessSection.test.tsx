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
    expect(html).toContain("never changes the source files or starts Claude Code");
    expect(html).toContain("Secrets excluded");
    expect(html).toContain("Not an encrypted backup");
  });

  it("requires conservative selection and a final confirmation before commit", async () => {
    openMock.mockResolvedValue("/mock/.claude");
    await act(async () => {
      root.render(<ImportHarnessSection onError={error => { throw new Error(error); }} />);
      await flush();
    });

    await act(async () => {
      button(container, "Choose Claude folder").click();
      await flush();
    });
    expect(container.textContent).toContain("Discovery metadata");
    expect(container.textContent).toContain("Nothing is selected by default");
    expect(button(container, "Preview selected").disabled).toBe(true);

    const artifacts = container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
    await act(async () => {
      artifacts[2].click();
      button(container, "Preview selected").click();
      await flush();
    });
    expect(container.textContent).toContain("Select what Bridge may import");
    expect(container.textContent).toContain("Historical Claude session");
    expect(button(container, "Review exact commit").disabled).toBe(true);

    await act(async () => {
      container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
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
