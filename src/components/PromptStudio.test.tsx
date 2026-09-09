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
  return { container, unmount: () => act(async () => { root.unmount(); container.remove(); }) };
}

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

/** The preview reloads via a second, cascading effect (stack loads, then the
 * preview fetch it triggers resolves via a real `crypto.subtle` digest) —
 * polling is more robust here than guessing a fixed flush count. */
async function waitFor(predicate: () => boolean, attempts = 20) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    if (predicate()) return;
    await flush();
  }
  throw new Error("waitFor: condition never became true");
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

function buttonWithText(container: HTMLElement, text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text);
}

async function clickOption(option: HTMLElement) {
  await act(async () => {
    option.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    option.click();
    await flush();
  });
}

/** Pick a prompt target from the header select. */
async function chooseTarget(container: HTMLElement, label: string) {
  const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Prompt target"]')!;
  await act(async () => { trigger.click(); await flush(); });
  const option = [...document.querySelectorAll<HTMLElement>('[role="listbox"][aria-label="Prompt target"] [role="option"]')]
    .find(node => node.textContent?.startsWith(label))!;
  await clickOption(option);
}

/** The list page's row for a section id: its mono id, tokens, and pills. */
function rowText(container: HTMLElement, id: string): string {
  const open = container.querySelector<HTMLButtonElement>(`button[aria-label="Edit ${id}"]`);
  return open?.parentElement?.textContent ?? "";
}

/** Open a section's editor from the list page. */
async function openSection(container: HTMLElement, id: string) {
  const open = container.querySelector<HTMLButtonElement>(`button[aria-label="Edit ${id}"]`)!;
  await act(async () => { open.click(); await flush(); });
}

/** Back to the list, via the breadcrumb's first crumb. */
async function backToList(container: HTMLElement) {
  const crumb = container.querySelector<HTMLButtonElement>('[aria-label="Breadcrumb"] button')!;
  await act(async () => { crumb.click(); await flush(); });
}

/** The save bar's Save, which replaced every per-card Save button. */
function saveButton(container: HTMLElement): HTMLButtonElement | undefined {
  return buttonWithText(container, "Save");
}

/** Edit a section, save it from the bar, and return to the list. */
async function editAndSave(container: HTMLElement, id: string, text: string) {
  await openSection(container, id);
  await typeInto(editorTextarea(container), text);
  await act(async () => { saveButton(container)!.click(); await flush(); });
  await backToList(container);
}

function fileInput(container: HTMLElement): HTMLInputElement {
  return container.querySelector<HTMLInputElement>('input[type="file"]')!;
}

async function selectFile(input: HTMLInputElement, content: string) {
  const file = new File([content], "overrides.json", { type: "application/json" });
  Object.defineProperty(input, "files", { value: [file], configurable: true });
  await act(async () => {
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();
    await flush();
  });
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
    expect(rowText(container, "bridge_role")).toContain("bridge_role");
    expect(rowText(container, "delegation_protocol")).toContain("delegation_protocol");

    await chooseTarget(container, "Research");
    expect(rowText(container, "worker_contract")).toContain("worker_contract");

    await chooseTarget(container, "Direct session");
    expect(container.textContent).toContain("nothing here for Bridge to override");
    await unmount();
  });

  // The list page has to answer "which of these did I change" without opening
  // any of them, which is the whole reason it is a list.
  it("section_rows_show_token_estimates_and_modified_badges", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const before = rowText(container, "delegation_protocol");
    expect(before).not.toContain("Modified");
    const beforeTokens = before.match(/(\d+) tok/)?.[1];

    await editAndSave(container, "delegation_protocol", "## Delegating work\nEmit one fenced bridge-delegate JSON object, with a good deal more text than the default so the token estimate visibly moves.");

    const after = rowText(container, "delegation_protocol");
    expect(after).toContain("Modified");
    expect(after.match(/(\d+) tok/)?.[1]).not.toBe(beforeTokens);
    await unmount();
  });

  it("editing_marks_dirty_and_save_persists_via_the_one_save_bar", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const saveSpy = vi.spyOn(bridgeApi, "savePromptSection");
    await openSection(container, "bridge_role");
    // A clean page has no save bar at all: there is nothing to save yet.
    expect(saveButton(container)).toBeUndefined();

    await typeInto(editorTextarea(container), "You are Bridge's customized orchestrator.");
    expect(saveButton(container)).toBeDefined();
    expect(container.textContent).toContain("Unsaved draft");

    await act(async () => { saveButton(container)!.click(); await flush(); });
    expect(saveSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", "You are Bridge's customized orchestrator.");
    expect(saveButton(container)).toBeUndefined();
    expect(container.textContent).not.toContain("Unsaved draft");

    await backToList(container);
    expect(rowText(container, "bridge_role")).toContain("Modified");
    await unmount();
  });

  it("keyboard_save_saves_the_draft", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const saveSpy = vi.spyOn(bridgeApi, "savePromptSection");
    await openSection(container, "bridge_role");

    await typeInto(editorTextarea(container), "Custom bridge role text.");
    await act(async () => {
      editorTextarea(container).dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true, bubbles: true }));
      await flush();
    });
    expect(saveSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", "Custom bridge role text.");
    await unmount();
  });

  // Drafts survive navigation, which is what makes a list-and-detail split safe
  // to click through.
  it("an_unsaved_draft_survives_going_back_to_the_list", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await openSection(container, "bridge_role");
    await typeInto(editorTextarea(container), "Half-written override.");
    await backToList(container);
    expect(rowText(container, "bridge_role")).toContain("Unsaved");

    await openSection(container, "bridge_role");
    expect(editorTextarea(container).value).toBe("Half-written override.");
    expect(saveButton(container)).toBeDefined();
    await unmount();
  });

  it("discarding_a_draft_puts_the_stored_text_back", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await openSection(container, "bridge_role");
    await typeInto(editorTextarea(container), "Text that will be thrown away.");
    await act(async () => { buttonWithText(container, "Discard")!.click(); await flush(); });
    expect(saveButton(container)).toBeUndefined();
    expect(editorTextarea(container).value).toContain("You are Bridge's starter orchestrator");
    await unmount();
  });

  it("per_section_reset_restores_default", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await editAndSave(container, "bridge_role", "Temporary override text.");
    expect(rowText(container, "bridge_role")).toContain("Modified");

    const resetSpy = vi.spyOn(bridgeApi, "resetPromptSection");
    await openSection(container, "bridge_role");
    await act(async () => { buttonWithText(container, "Reset bridge_role")!.click(); await flush(); });
    expect(resetSpy).toHaveBeenCalledWith("orchestrator", "bridge_role");
    expect(editorTextarea(container).value).toContain("You are Bridge's starter orchestrator");

    await backToList(container);
    expect(rowText(container, "bridge_role")).not.toContain("Modified");
    await unmount();
  });

  it("whole_target_reset_resets_only_this_target", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();

    await editAndSave(container, "bridge_role", "Overridden bridge role.");
    await editAndSave(container, "delegation_protocol", "## Delegating work\nEmit one fenced bridge-delegate JSON object, edited.");

    await chooseTarget(container, "Research");
    await editAndSave(container, "worker_contract", "Custom research contract.");
    expect(rowText(container, "worker_contract")).toContain("Modified");

    await chooseTarget(container, "Orchestrator");
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await act(async () => { buttonWithText(container, "Reset all for Orchestrator")!.click(); await flush(); });
    expect(rowText(container, "bridge_role")).not.toContain("Modified");
    expect(rowText(container, "delegation_protocol")).not.toContain("Modified");

    await chooseTarget(container, "Research");
    expect(rowText(container, "worker_contract")).toContain("Modified");
    await unmount();
  });

  it("lint_warning_appears_before_save", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await openSection(container, "delegation_protocol");
    await typeInto(editorTextarea(container), "## Delegating work\nJust emit JSON, no fenced marker mentioned here.");

    expect(container.textContent).toContain("bridge-delegate");
    expect(container.textContent).toContain("missing");
    // A warning explains, it does not block: the save bar is still offered.
    expect(saveButton(container)!.disabled).toBe(false);
    await unmount();
  });

  // The preview describes the target, not any one section, so it lives on the
  // list page — which is also the only page a direct session ever has.
  it("preview_splits_exact_envelopes_from_provider_layers", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await waitFor(() => container.textContent!.includes("Exact Bridge bytes"));

    const expected = await bridgeApi.previewCompiledPrompt("orchestrator");
    expect(container.textContent).toContain("Exact Bridge bytes");
    expect(container.textContent).toContain(expected.prefixHash);
    expect(container.textContent).toContain(expected.prefixId);
    expect(container.textContent).toContain(String(expected.prefixBytes));
    expect(container.textContent).toContain(expected.stablePrefix);
    expect(container.textContent).toContain(expected.variableSuffix);

    expect(container.textContent).toContain("Provider layers (not exact)");
    for (const layer of expected.providerLayers) {
      expect(container.textContent).toContain(layer.adapter);
      expect(container.textContent).toContain("unavailable");
      expect(container.textContent).toContain(layer.detail);
    }

    // Provider-owned detail never lands inside the exact-bytes block itself.
    const [stablePrefixPre] = container.querySelectorAll("pre");
    expect(stablePrefixPre.textContent).not.toContain(expected.providerLayers[0].detail);
    await unmount();
  });

  it("a_direct_session_still_gets_its_compiled_preview", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await waitFor(() => container.textContent!.includes("Exact Bridge bytes"));
    await chooseTarget(container, "Direct session");
    expect(container.textContent).toContain("nothing here for Bridge to override");
    await waitFor(() => container.textContent!.includes("Provider layers (not exact)"));
    await unmount();
  });

  it("cache_impact_tracks_prefix_hash_changes", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await waitFor(() => container.textContent!.includes("Exact Bridge bytes"));
    expect(container.textContent).not.toContain("Bridge prefix changed");

    await editAndSave(container, "bridge_role", "You are Bridge's customized orchestrator, with materially different text so the compiled prefix hash changes.");
    await waitFor(() => container.textContent!.includes("Bridge prefix changed"));

    expect(container.textContent).toContain("estimated");
    await unmount();
  });

  it("import_rejects_malformed_files_and_applies_valid_entries", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();
    const saveSpy = vi.spyOn(bridgeApi, "savePromptSection");

    await selectFile(fileInput(container), JSON.stringify([1, 2, 3]));
    expect(container.textContent).toMatch(/JSON object/);
    expect(saveSpy).not.toHaveBeenCalled();
    // Failures are announced through the same live region as successes.
    const liveRegion = () => container.querySelector('[aria-live="polite"]')!;
    expect(liveRegion().textContent).toContain("Failed:");

    await selectFile(fileInput(container), JSON.stringify({ "not-a-real-target": { bridge_role: { state: "overridden", text: "x" } } }));
    expect(container.textContent).toMatch(/Unknown prompt target/);
    expect(saveSpy).not.toHaveBeenCalled();

    await selectFile(fileInput(container), JSON.stringify({ orchestrator: { not_a_real_section: { state: "overridden", text: "x" } } }));
    expect(container.textContent).toMatch(/Unknown section/);
    expect(saveSpy).not.toHaveBeenCalled();

    await selectFile(fileInput(container), JSON.stringify({ orchestrator: { bridge_role: { state: "overridden" } } }));
    expect(container.textContent).toMatch(/Malformed override entry/);
    expect(saveSpy).not.toHaveBeenCalled();
    expect(rowText(container, "bridge_role")).not.toContain("Modified");

    await selectFile(fileInput(container), JSON.stringify({ orchestrator: { bridge_role: { state: "overridden", text: "Imported bridge role text." } } }));
    expect(saveSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", "Imported bridge role text.");
    expect(rowText(container, "bridge_role")).toContain("Modified");
    await unmount();
  });

  it("export_downloads_one_json_file", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();

    await editAndSave(container, "bridge_role", "Exported override text.");
    await flush();

    const createObjectURL = vi.fn((_blob: unknown) => "blob:mock-url");
    const revokeObjectURL = vi.fn((_url: unknown) => undefined);
    const originalCreateObjectURL = URL.createObjectURL;
    const originalRevokeObjectURL = URL.revokeObjectURL;
    URL.createObjectURL = createObjectURL;
    URL.revokeObjectURL = revokeObjectURL;
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);

    try {
      await act(async () => { buttonWithText(container, "Export overrides")!.click(); await flush(); });

      expect(clickSpy).toHaveBeenCalledTimes(1);
      expect(createObjectURL).toHaveBeenCalledTimes(1);
      expect(revokeObjectURL).toHaveBeenCalledTimes(1);
      const blob = createObjectURL.mock.calls[0][0] as Blob;
      expect(blob.type).toBe("application/json");
      // jsdom's Blob predates .text(); FileReader is the portable read.
      const payload = JSON.parse(await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error);
        reader.readAsText(blob);
      }));
      expect(payload.orchestrator.bridge_role).toEqual({ state: "overridden", text: "Exported override text." });
    } finally {
      URL.createObjectURL = originalCreateObjectURL;
      URL.revokeObjectURL = originalRevokeObjectURL;
    }
    await unmount();
  });

  it("controls_have_accessible_names_and_live_region_announces_saves", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();

    expect(container.querySelector('button[aria-label="Prompt target"]')).not.toBeNull();
    expect(container.querySelector('input[aria-label="Import prompt overrides"]')).not.toBeNull();
    expect(buttonWithText(container, "Export overrides")).toBeDefined();
    expect(container.textContent).toContain("Compiled preview");

    const live = container.querySelector('[aria-live="polite"]')!;
    expect(live.textContent).toBe("");

    await openSection(container, "bridge_role");
    await typeInto(editorTextarea(container), "Accessible save text.");
    await act(async () => { saveButton(container)!.click(); await flush(); });
    expect(container.querySelector('[aria-live="polite"]')!.textContent).toContain("Saved bridge_role");
    await unmount();
  });

  it("every_target_is_offered_exactly_once", async () => {
    // Locks the exact set the target select renders. A future target id that no
    // group matched used to vanish from the old rail silently; the select is
    // now the whole list, so this pins it.
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Prompt target"]')!;
    await act(async () => { trigger.click(); await flush(); });
    const labels = [...document.querySelectorAll<HTMLElement>('[role="listbox"][aria-label="Prompt target"] [role="option"]')]
      .map(node => node.textContent?.replace("Has overrides", "").trim());
    expect(labels).toEqual(["Orchestrator", "Research", "Implementation", "Verification", "Planning", "Documentation", "Direct session"]);
    await unmount();
  });

  it("override_note_appears_on_a_non_active_target_after_a_save", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();

    const optionFor = (label: string) => [...document.querySelectorAll<HTMLElement>('[role="listbox"][aria-label="Prompt target"] [role="option"]')]
      .find(node => node.textContent?.startsWith(label));
    const openTargets = async () => {
      const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Prompt target"]')!;
      await act(async () => { trigger.click(); await flush(); });
    };

    await openTargets();
    expect(optionFor("Research")!.textContent).not.toContain("Has overrides");
    await clickOption(optionFor("Research")!);

    await editAndSave(container, "worker_contract", "Custom research contract.");

    await chooseTarget(container, "Orchestrator");
    await openTargets();
    expect(optionFor("Research")!.textContent).toContain("Has overrides");
    await unmount();
  });

  it("mount_time_stack_load_does_not_clobber_a_save_that_lands_first", async () => {
    // The mount effect fetches every target's stack once (for the target
    // select's override notes) via Promise.all(TARGETS.map(...)) — 7 calls,
    // fired synchronously in TARGETS order before anything else awaits. Every
    // call after that is the per-target effect / a mutation handler. Holding
    // the first 7 calls back and releasing them only after a save has landed
    // reproduces the exact race: a stale snapshot resolving after a fresh save
    // must not overwrite it.
    const TARGETS_COUNT = 7;
    const original = bridgeApi.promptStack.bind(bridgeApi);
    let callIndex = 0;
    const releaseMountBatch: (() => void)[] = [];
    vi.spyOn(bridgeApi, "promptStack").mockImplementation((target, depth) => {
      callIndex += 1;
      const real = original(target, depth);
      if (callIndex <= TARGETS_COUNT) {
        return new Promise(resolve => { releaseMountBatch.push(() => { void real.then(resolve); }); });
      }
      return real;
    });

    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();

    await editAndSave(container, "bridge_role", "Saved before the mount batch resolves.");
    expect(rowText(container, "bridge_role")).toContain("Modified");

    await act(async () => {
      releaseMountBatch.forEach(release => release());
      await flush();
      await flush();
    });

    expect(rowText(container, "bridge_role")).toContain("Modified");
    await unmount();
  });

  it("revision_history_lists_operations_and_restore_works", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    await flush();

    await openSection(container, "bridge_role");
    await typeInto(editorTextarea(container), "First override of bridge role.");
    await act(async () => { saveButton(container)!.click(); await flush(); });
    await flush();

    await typeInto(editorTextarea(container), "Second override of bridge role.");
    await act(async () => { saveButton(container)!.click(); await flush(); });
    await flush();

    expect(container.textContent).toContain("History");
    expect(container.textContent).toContain("override");
    const restoreButtons = [...container.querySelectorAll<HTMLButtonElement>('button[aria-label^="Restore bridge_role to revision"]')];
    expect(restoreButtons.length).toBeGreaterThanOrEqual(2);
    // History renders most-recent first: index 0 is "Second override", index 1
    // is "First override" — restoring index 1 should bring the older text back.
    const olderRestoreButton = restoreButtons[1];
    const restoreSpy = vi.spyOn(bridgeApi, "restorePromptRevision");

    await act(async () => { olderRestoreButton.click(); await flush(); });
    await flush();

    expect(restoreSpy).toHaveBeenCalledWith("orchestrator", "bridge_role", expect.any(Number));
    expect(container.textContent).toContain("restore");
    expect(editorTextarea(container).value).toContain("First override of bridge role.");
    await unmount();
  });

  it("shows approved proposal attribution on shared guidance and restores an earlier revision", async () => {
    const earlier = await bridgeApi.savePromptSection("orchestrator", "additional_guidance", "Earlier guidance.");
    const latest = await bridgeApi.savePromptSection("orchestrator", "additional_guidance", "Earlier guidance.\n\nApproved additional guidance.");
    const originalStack = bridgeApi.promptStack;
    vi.spyOn(bridgeApi, "promptStack").mockImplementation(async (target, depth) => {
      const stack = await originalStack(target, depth);
      if (target !== "orchestrator") return stack;
      return { ...stack, sections: stack.sections.map(section => section.id !== "additional_guidance" ? section : {
        ...section,
        revisions: section.revisions.map(revision => revision.id !== latest.revision.id ? revision : {
          ...revision,
          attribution: { actorSessionId: "actor-session", actorTurnId: "actor-turn", actorRole: "orchestrator", proposalId: "proposal-123", rationale: "Keep <b>reliable</b> lessons from earlier work." },
        }),
      }) };
    });
    const { container, unmount } = await mount(<PromptStudio />);
    await act(async () => waitFor(() => !!container.querySelector('[aria-label="Edit additional_guidance"]')));
    expect(rowText(container, "additional_guidance")).toContain("Shared role guidance");
    await openSection(container, "additional_guidance");
    expect(container.textContent).toContain("Applies at the next start or relaunch");
    expect(container.textContent).toContain("Proposed by orchestrator · session actor-session · approved by you");
    expect(container.textContent).toContain("Keep <b>reliable</b> lessons from earlier work.");
    expect(container.textContent).toContain("Proposal proposal-123");
    expect(container.querySelector("b")).toBeNull();
    const restoreSpy = vi.spyOn(bridgeApi, "restorePromptRevision");
    const restore = container.querySelector<HTMLButtonElement>(`button[aria-label="Restore additional_guidance to revision ${earlier.revision.id} (override)"]`)!;
    await act(async () => { restore.click(); await flush(); });
    expect(restoreSpy).toHaveBeenCalledWith("orchestrator", "additional_guidance", earlier.revision.id);
    expect(editorTextarea(container).value).toBe("Earlier guidance.");
    await unmount();
  });

  it("uses_no_native_select_and_no_native_checkbox", async () => {
    const { container, unmount } = await mount(<PromptStudio />);
    await flush();
    expect(container.querySelectorAll("select")).toHaveLength(0);
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    await openSection(container, "bridge_role");
    expect(container.querySelectorAll("select")).toHaveLength(0);
    await unmount();
  });
});
