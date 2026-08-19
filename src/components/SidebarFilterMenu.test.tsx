// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_CHAT_VIEW, type ChatView } from "./sidebarChats";
import { SidebarFilterMenu } from "./SidebarFilterMenu";

let container: HTMLDivElement;
let root: Root;

const agents = [{ id: "claude", label: "Claude" }, { id: "codex", label: "Codex" }];

function mount(view: ChatView = DEFAULT_CHAT_VIEW, onChange: (next: ChatView) => void = () => {}, allowProjectGrouping = true) {
  act(() => {
    root.render(<SidebarFilterMenu view={view} agents={agents} allowProjectGrouping={allowProjectGrouping} onChange={onChange} />);
  });
}

const trigger = () => container.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]')!;
const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
const click = (element: Element) => {
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};
const rowByLabel = (label: string) =>
  [...menu()!.querySelectorAll("button")].find(button => button.textContent?.startsWith(label))!;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("SidebarFilterMenu", () => {
  it("stays closed until the trigger is pressed", () => {
    mount();
    expect(menu()).toBeNull();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    click(trigger());
    expect(menu()).not.toBeNull();
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
  });

  it("lists the four rows with their current values", () => {
    mount({ status: "active", agent: "codex", groupBy: "project", sortBy: "name" });
    click(trigger());
    const text = menu()!.textContent ?? "";
    for (const fragment of ["Status", "Active", "Agent", "Codex", "Group by", "Project", "Sort by", "Name"]) {
      expect(text).toContain(fragment);
    }
  });

  it("drills into a row and marks the current option", () => {
    mount({ ...DEFAULT_CHAT_VIEW, groupBy: "agent" });
    click(trigger());
    click(rowByLabel("Group by"));
    const options = [...menu()!.querySelectorAll('[role="menuitemradio"]')];
    expect(options.map(option => option.textContent)).toEqual(["Date", "Project", "Agent", "Status", "None"]);
    expect(options.filter(option => option.getAttribute("aria-checked") === "true").map(option => option.textContent)).toEqual(["Agent"]);
  });

  it("withholds project grouping where nothing has a project", () => {
    mount(DEFAULT_CHAT_VIEW, () => {}, false);
    click(trigger());
    click(rowByLabel("Group by"));
    expect([...menu()!.querySelectorAll('[role="menuitemradio"]')].map(option => option.textContent)).toEqual(["Date", "Agent", "Status", "None"]);
  });

  it("reports a choice and returns to the root panel", () => {
    const onChange = vi.fn();
    mount(DEFAULT_CHAT_VIEW, onChange);
    click(trigger());
    click(rowByLabel("Status"));
    click([...menu()!.querySelectorAll('[role="menuitemradio"]')].find(option => option.textContent === "Waiting")!);
    expect(onChange).toHaveBeenCalledWith({ ...DEFAULT_CHAT_VIEW, status: "waiting" });
    // Back on the root panel, so a second setting can be changed in one visit.
    expect(menu()!.querySelector('[role="menuitem"]')).not.toBeNull();
  });

  it("offers every present harness plus All on the agent panel", () => {
    mount();
    click(trigger());
    click(rowByLabel("Agent"));
    expect([...menu()!.querySelectorAll('[role="menuitemradio"]')].map(option => option.textContent)).toEqual(["All", "Claude", "Codex"]);
  });

  it("closes on Escape and on an outside pointer press", () => {
    mount();
    click(trigger());
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(menu()).toBeNull();

    click(trigger());
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(menu()).toBeNull();
  });

  it("marks the trigger active while a filter narrows the list", () => {
    mount();
    expect(trigger().className).toContain("text-muted-foreground");
    mount({ ...DEFAULT_CHAT_VIEW, status: "failed" });
    expect(trigger().className).toContain("bg-accent");
  });
});
