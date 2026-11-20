// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatFileChange } from "../chatChangedFiles";
import { COMPOSER_MENTION_TRIGGERS, FileMentionMenu } from "./FileMentionMenu";

let container: HTMLDivElement;
let root: Root;

const FILES = [
  "src-tauri/bridge-core/src/policy.rs",
  "src/components/PolicyCard.tsx",
  "docs/delegation-policy.md",
  "POLICY.md",
];

async function mount(overrides: Partial<Parameters<typeof FileMentionMenu>[0]> = {}) {
  const props: Parameters<typeof FileMentionMenu>[0] = {
    id: "file-mention-listbox",
    optionIdPrefix: "file-mention-option-",
    files: FILES,
    query: "pol",
    activeIndex: 0,
    onActiveIndexChange: vi.fn(),
    onPick: vi.fn(),
    ...overrides,
  };
  await act(async () => root.render(<FileMentionMenu {...props} />));
  return props;
}

const rows = () => [...container.querySelectorAll<HTMLButtonElement>('[role="option"]')];
const marked = (element: Element | null) => [...(element?.querySelectorAll("mark") ?? [])].map(mark => mark.textContent);

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("FileMentionMenu", () => {
  it("draws the name first in mono with the match marked, and the folder dimmed beside it", async () => {
    await mount();
    const [policy, card, doc, rootFile] = rows();

    const name = policy.querySelector("[data-mention-name]")!;
    expect(name.textContent).toBe("policy.rs");
    expect(name.className).toContain("font-mono");
    expect(name.className).toContain("text-foreground");
    expect(marked(name)).toEqual(["pol"]);
    const folder = policy.querySelector("[data-mention-folder]")!;
    expect(folder.textContent).toBe("src-tauri/bridge-core/src");
    expect(folder.className).toContain("text-faint");
    expect(marked(folder)).toEqual([]);

    // Case-insensitive, and wherever the name holds it.
    expect(marked(card.querySelector("[data-mention-name]"))).toEqual(["Pol"]);
    expect(marked(doc.querySelector("[data-mention-name]"))).toEqual(["pol"]);
    expect(doc.querySelector("[data-mention-name]")!.textContent).toBe("delegation-policy.md");

    // A root file has a name and no folder.
    expect(rootFile.querySelector("[data-mention-name]")!.textContent).toBe("POLICY.md");
    expect(rootFile.querySelector("[data-mention-folder]")).toBeNull();
  });

  it("marks the match without colour: weight and underline on the row's own ink", async () => {
    await mount();
    const mark = rows()[0].querySelector("mark")!;
    expect(mark.className).toContain("underline");
    expect(mark.className).toContain("font-semibold");
    expect(mark.className).toContain("bg-transparent");
    expect(mark.className).toContain("text-inherit");
  });

  it("marks a folder-only match in the folder", async () => {
    await mount({ files: ["src-tauri/bridge-core/src/lib.rs"], query: "bridge" });
    const [row] = rows();
    expect(marked(row.querySelector("[data-mention-name]"))).toEqual([]);
    expect(marked(row.querySelector("[data-mention-folder]"))).toEqual(["bridge"]);
  });

  it("puts M or A on the right of files this chat changed, and a legend in the header", async () => {
    const changes = new Map<string, ChatFileChange>([
      ["src-tauri/bridge-core/src/policy.rs", "M"],
      ["docs/delegation-policy.md", "A"],
    ]);
    await mount({ changes });
    const [policy, card, doc] = rows();
    const modified = policy.querySelector("[data-mention-change]")!;
    expect(modified.textContent).toBe("M");
    expect(modified.className).toContain("text-info");
    expect(policy.getAttribute("aria-label")).toBe("src-tauri/bridge-core/src/policy.rs, changed in this chat");
    expect(doc.querySelector("[data-mention-change]")!.textContent).toBe("A");
    expect(doc.getAttribute("aria-label")).toBe("docs/delegation-policy.md, added in this chat");
    expect(card.querySelector("[data-mention-change]")).toBeNull();
    expect(card.getAttribute("aria-label")).toBe("src/components/PolicyCard.tsx");

    expect(container.querySelector("[data-mention-legend]")!.textContent).toBe("MAchanged in this chat");
  });

  it("draws no markers and no legend when the chat changed none of the listed files", async () => {
    await mount({ changes: new Map([["src/elsewhere.ts", "M"]]) });
    expect(container.querySelector("[data-mention-change]")).toBeNull();
    expect(container.querySelector("[data-mention-legend]")).toBeNull();
  });

  it("names what is listed in the header and teaches keys and triggers in the footer", async () => {
    await mount({ triggers: COMPOSER_MENTION_TRIGGERS });
    const listbox = container.querySelector('[role="listbox"]')!;
    expect(listbox.id).toBe("file-mention-listbox");
    expect(listbox.textContent).toContain("Files");
    expect(listbox.textContent).toContain("matching “pol”");
    const footer = listbox.lastElementChild!;
    expect(footer.getAttribute("aria-hidden")).toBe("true");
    expect([...footer.querySelectorAll("kbd")].map(kbd => kbd.textContent)).toEqual(["↵", "#", "/", "$"]);
    expect(footer.textContent).toBe("↵Insert#agents/commands$provider");
  });

  it("says only Files when nothing is typed yet", async () => {
    await mount({ query: "" });
    expect(container.textContent).not.toContain("matching");
    expect(marked(container)).toEqual([]);
  });

  it("keeps the option ids, selection and pick the composer's keyboard relies on", async () => {
    const props = await mount({ activeIndex: 1 });
    const options = rows();
    expect(options.map(option => option.id)).toEqual(FILES.map((_, index) => `file-mention-option-${index}`));
    expect(options.map(option => option.getAttribute("aria-selected"))).toEqual(["false", "true", "false", "false"]);
    expect(options[1].className).toContain("bg-accent");
    expect(options.map(option => option.dataset.mentionIndex)).toEqual(["0", "1", "2", "3"]);

    await act(async () => { options[2].dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(props.onActiveIndexChange).toHaveBeenCalledWith(2);

    const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    await act(async () => { options[2].dispatchEvent(press); });
    // Keeps focus in the composer: the press never moves it to the row.
    expect(press.defaultPrevented).toBe(true);
    expect(props.onPick).toHaveBeenCalledWith("docs/delegation-policy.md");
  });
});
