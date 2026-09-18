// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ClaudeHarnessSettings } from "./ClaudeHarnessSettings";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it("shows the stored source and says that no token is stored", () => {
  const html = renderToStaticMarkup(<ClaudeHarnessSettings
    value={{ workerCredentialSource: "environment" }}
    disabled={false}
    onChange={() => undefined}
  />);
  expect(html).toContain("Environment only");
  expect(html).toContain("never stores the token");
  expect(html).not.toContain('type="password"');
});

it("defaults to Automatic and reports a change once, as the source alone", async () => {
  const onChange = vi.fn();
  await act(async () => { root.render(<ClaudeHarnessSettings value={{}} disabled={false} onChange={onChange} />); });
  const trigger = host.querySelector<HTMLButtonElement>('button[aria-haspopup="listbox"]')!;
  expect(trigger.textContent).toContain("Automatic");
  await act(async () => trigger.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
    .find(item => item.textContent?.includes("Environment only"))!;
  expect(option).toBeTruthy();
  await act(async () => {
    option.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    option.click();
  });
  expect(onChange.mock.calls).toEqual([[{ workerCredentialSource: "environment" }]]);
});
