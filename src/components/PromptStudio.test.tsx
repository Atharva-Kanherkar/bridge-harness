// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { PromptStudio } from "./PromptStudio";

// CodeMirror owns a real DOM and its own measurement loop, neither of which
// jsdom provides usefully — same stand-in CodePanel.test.tsx uses for the
// same reason. What this file checks is PromptStudio's wiring around it.
vi.mock("./editor/CodeEditor", () => ({
  CodeEditor: ({ docKey, doc, onChange, onSave }: {
    docKey: string; doc: string; onChange: (value: string) => void; onSave: () => void;
  }) => <textarea
    data-testid="editor"
    data-dockey={docKey}
    defaultValue={doc}
    onChange={event => onChange(event.target.value)}
    onKeyDown={event => { if (event.metaKey && event.key === "s") onSave(); }}
  />,
}));

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => root.unmount()) };
}

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

/** React listens for `input` via its own value tracker, so a bare
 *  `element.value = x` is invisible to it. */
async function typeInto(element: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function editorTextarea(container: HTMLElement): HTMLTextAreaElement {
  return container.querySelector<HTMLTextAreaElement>('[data-testid="editor"]')!;
}

function navButton(container: HTMLElement, label: string): HTMLButtonElement {
  return [...container.querySelectorAll<HTMLButtonElement>('nav[aria-label="Prompt targets"] button')]
    .find(node => node.textContent === label)!;
}

function optionButton(container: HTMLElement, id: string): HTMLButtonElement {
  return [...container.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    .find(node => node.querySelector(".font-mono")?.textContent === id)!;
}

function optionText(container: HTMLElement, id: string): string {
  return optionButton(container, id)?.textContent ?? "";
}

function buttonWithText(container: HTMLElement, text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text);
}

beforeEach(async () => {
  // Prompt Studio's browser-mode overrides live in a module-level map inside
  // src/api.ts; reset it between tests the same way a settings-wide reset
  // does, so one test's saved override cannot leak into the next.
  await bridgeApi.resetAllConfig();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("PromptStudio", () => {
  it("renders_target_specific_stacks", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    expect(optionText(container, "bridge_role")).toContain("bridge_role");
    expect(optionText(container, "delegation_protocol")).toContain("delegation_protocol");

    await act(async () => { navButton(container, "Research").click(); });
    await flush();
    expect(optionText(container, "worker_contract")).toContain("worker_contract");

    await act(async () => { navButton(container, "Direct session").click(); });
    await flush();
    expect(container.textContent).toContain("nothing here for Bridge to override");
    await unmount();
  });

  it("section_rows_show_token_estimates_and_modified_badges", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await act(async () => { optionButton(container, "delegation_protocol").click(); });
    const before = optionText(container, "delegation_protocol");
    expect(before).not.toContain("Modified");
    const beforeTokens = before.match(/(\d+) tok/)?.[1];

    await typeInto(editorTextarea(container), "## Delegating work\nEmit one fenced bridge-delegate JSON object, with a good deal more text than the default so the token estimate visibly moves.");
    await act(async () => { buttonWithText(container, "Save delegation_protocol")!.click(); await flush(); });

    const after = optionText(container, "delegation_protocol");
    expect(after).toContain("Modified");
    expect(after.match(/(\d+) tok/)?.[1]).not.toBe(beforeTokens);
    await unmount();
  });

  it("editing_marks_dirty_and_save_persists_via_api", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const saveSpy = vi.spyOn(bridgeApi, "savePromptSection");
    const save = () => buttonWithText(container, "Save bridge_role")!;
    expect(save().disabled).toBe(true);

    await typeInto(editorTextarea(container), "You are Bridge's customized orchestrator.");
    expect(save().disabled).toBe(false);

    await act(async () => { save().click(); await flush(); });
    expect(saveSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", "You are Bridge's customized orchestrator.");
    expect(save().disabled).toBe(true);
    expect(optionText(container, "bridge_role")).toContain("Modified");
    await unmount();
  });

  it("keyboard_save_saves_the_draft", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const saveSpy = vi.spyOn(bridgeApi, "savePromptSection");

    await typeInto(editorTextarea(container), "Custom bridge role text.");
    await act(async () => {
      editorTextarea(container).dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true, bubbles: true }));
      await flush();
    });
    expect(saveSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", "Custom bridge role text.");
    await unmount();
  });

  it("per_section_reset_restores_default", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await typeInto(editorTextarea(container), "Temporary override text.");
    await act(async () => { buttonWithText(container, "Save bridge_role")!.click(); await flush(); });
    expect(optionText(container, "bridge_role")).toContain("Modified");

    const resetSpy = vi.spyOn(bridgeApi, "resetPromptSection");
    await act(async () => { buttonWithText(container, "Reset bridge_role")!.click(); await flush(); });
    expect(resetSpy).toHaveBeenCalledWith("orchestrator", "bridge_role");
    expect(optionText(container, "bridge_role")).not.toContain("Modified");
    expect(editorTextarea(container).value).toContain("You are Bridge's starter orchestrator");
    await unmount();
  });

  it("whole_target_reset_resets_only_this_target", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();

    await typeInto(editorTextarea(container), "Overridden bridge role.");
    await act(async () => { buttonWithText(container, "Save bridge_role")!.click(); await flush(); });
    await act(async () => { optionButton(container, "delegation_protocol").click(); });
    await typeInto(editorTextarea(container), "## Delegating work\nEmit one fenced bridge-delegate JSON object, edited.");
    await act(async () => { buttonWithText(container, "Save delegation_protocol")!.click(); await flush(); });

    await act(async () => { navButton(container, "Research").click(); await flush(); });
    await typeInto(editorTextarea(container), "Custom research contract.");
    await act(async () => { buttonWithText(container, "Save worker_contract")!.click(); await flush(); });
    expect(optionText(container, "worker_contract")).toContain("Modified");

    await act(async () => { navButton(container, "Orchestrator").click(); await flush(); });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await act(async () => { buttonWithText(container, "Reset all for Orchestrator")!.click(); await flush(); });
    expect(optionText(container, "bridge_role")).not.toContain("Modified");
    expect(optionText(container, "delegation_protocol")).not.toContain("Modified");

    await act(async () => { navButton(container, "Research").click(); await flush(); });
    expect(optionText(container, "worker_contract")).toContain("Modified");
    await unmount();
  });

  it("lint_warning_appears_before_save", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await act(async () => { optionButton(container, "delegation_protocol").click(); });
    await typeInto(editorTextarea(container), "## Delegating work\nJust emit JSON, no fenced marker mentioned here.");

    expect(container.textContent).toContain("bridge-delegate");
    expect(container.textContent).toContain("missing");
    expect(buttonWithText(container, "Save delegation_protocol")!.disabled).toBe(false);
    await unmount();
  });
});
