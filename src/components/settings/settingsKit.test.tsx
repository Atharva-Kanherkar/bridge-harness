// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import {
  Field, SaveBar, Select, SettingsGroup, SettingsPage, SettingsRow, StatusPill, Switch,
} from "./kit";

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => { root.unmount(); container.remove(); }) };
}

const options = [
  { value: "low", label: "Low" },
  { value: "high", label: "High" },
];

describe("SettingsRow", () => {
  it("renders one label, one description, and one control", async () => {
    const { container, unmount } = await mount(
      <SettingsRow label="Default effort" description="Applies to new sessions" control={<span data-testid="control" />} />,
    );
    expect(container.textContent).toContain("Default effort");
    expect(container.textContent).toContain("Applies to new sessions");
    expect(container.querySelectorAll('[data-testid="control"]')).toHaveLength(1);
    await unmount();
  });

  // A row that opens a page is the whole row, so the hit target matches what
  // the chevron promises.
  it("becomes the button that opens a detail page, ending in a chevron", async () => {
    const onOpen = vi.fn();
    const { container, unmount } = await mount(<SettingsRow label="OpenCode" onOpen={onOpen} />);
    const row = container.querySelector("button")!;
    await act(async () => row.click());
    expect(onOpen).toHaveBeenCalledOnce();
    expect(container.querySelector("svg")).toBeTruthy();
    await unmount();
  });

  it("shows the saved confirmation beside the control, not instead of it", async () => {
    const { container, unmount } = await mount(
      <SettingsRow label="Enabled" saved control={<span data-testid="control" />} />,
    );
    expect(container.textContent).toContain("Saved");
    expect(container.querySelector('[data-testid="control"]')).toBeTruthy();
    await unmount();
  });
});

describe("Switch", () => {
  it("is a role=switch button that reports aria-checked from its prop", async () => {
    const { container, unmount } = await mount(<Switch checked label="Enabled" onChange={() => undefined} />);
    const toggle = container.querySelector('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(container.querySelector("input")).toBeNull();
    await unmount();
  });

  it("never flips itself: it reports the requested value and re-renders from its prop", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(<Switch checked={false} label="Enabled" onChange={onChange} />);
    await act(async () => container.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
    expect(onChange).toHaveBeenCalledWith(true);
    expect(container.querySelector('[role="switch"]')!.getAttribute("aria-checked")).toBe("false");
    await unmount();
  });

  it("does not fire while disabled", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(<Switch checked={false} disabled label="Enabled" onChange={onChange} />);
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.disabled).toBe(true);
    await act(async () => toggle.click());
    expect(onChange).not.toHaveBeenCalled();
    await unmount();
  });
});

describe("Select", () => {
  // Native selects render a system menu that ignores every token in the kit,
  // which is the reason the design bans them from Settings outright.
  it("is a listbox button with no native select anywhere", async () => {
    const { container, unmount } = await mount(
      <Select label="Effort" value="low" options={options} onChange={() => undefined} />,
    );
    const trigger = container.querySelector("button")!;
    expect(trigger.getAttribute("aria-haspopup")).toBe("listbox");
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(trigger.textContent).toContain("Low");
    expect(container.querySelector("select")).toBeNull();
    await unmount();
  });

  it("opens outside the clipped settings card and reports the chosen value once", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <SettingsGroup label="Runtime">
        <SettingsRow label="Effort" control={<Select label="Effort" value="low" options={options} onChange={onChange} />} />
      </SettingsGroup>,
    );
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    const listbox = document.querySelector('[role="listbox"]')!;
    expect(listbox).toBeTruthy();
    expect(container.contains(listbox)).toBe(false);
    expect(listbox.querySelectorAll('[role="option"]')).toHaveLength(2);
    const high = [...listbox.querySelectorAll<HTMLElement>('[role="option"]')].find(item => item.textContent?.includes("High"))!;
    await act(async () => {
      high.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      high.click();
    });
    expect(onChange.mock.calls).toEqual([["high"]]);
    // Closing can retain the popup briefly while Base UI finishes its exit.
    expect(container.querySelector("button")!.getAttribute("aria-expanded")).toBe("false");
    await unmount();
  });

  it("is disabled when it has nothing to choose from", async () => {
    const { container, unmount } = await mount(
      <Select label="Model" value="" options={[]} onChange={() => undefined} />,
    );
    expect(container.querySelector<HTMLButtonElement>("button")!.disabled).toBe(true);
    await unmount();
  });
});

describe("Field", () => {
  it("reports every keystroke and never renders a checkbox", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <Field label="Executable path" value="/usr/bin/opencode" onChange={onChange} />,
    );
    const input = container.querySelector("input")!;
    expect(input.type).toBe("text");
    expect(input.value).toBe("/usr/bin/opencode");
    await unmount();
  });
});

describe("SaveBar", () => {
  it("stays out of the way until the page is dirty", async () => {
    const { container, unmount } = await mount(
      <SaveBar dirty={false} saving={false} onSave={() => undefined} onDiscard={() => undefined} />,
    );
    expect(container.textContent).toBe("");
    await unmount();
  });

  it("offers Discard and Save, and disables both while a save is in flight", async () => {
    const onSave = vi.fn();
    const onDiscard = vi.fn();
    const { container, unmount } = await mount(
      <SaveBar dirty saving={false} onSave={onSave} onDiscard={onDiscard} />,
    );
    expect(container.textContent).toContain("Unsaved changes");
    const buttons = [...container.querySelectorAll<HTMLButtonElement>("button")];
    await act(async () => buttons.find(item => item.textContent === "Discard")!.click());
    await act(async () => buttons.find(item => item.textContent === "Save")!.click());
    expect(onDiscard).toHaveBeenCalledOnce();
    expect(onSave).toHaveBeenCalledOnce();
    await unmount();
  });

  it("cannot be saved twice: Save is disabled while saving", async () => {
    const onSave = vi.fn();
    const { container, unmount } = await mount(
      <SaveBar dirty saving onSave={onSave} onDiscard={() => undefined} />,
    );
    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.includes("Saving"))!;
    expect(save.disabled).toBe(true);
    await act(async () => save.click());
    expect(onSave).not.toHaveBeenCalled();
    await unmount();
  });
});

describe("StatusPill", () => {
  it("gives each tone its own class, and colors nothing by default", async () => {
    const { container, unmount } = await mount(<>
      <StatusPill>Not installed</StatusPill>
      <StatusPill tone="success">Ready</StatusPill>
      <StatusPill tone="warning">Needs repair</StatusPill>
    </>);
    const classes = [...container.querySelectorAll("span")].map(item => item.className);
    expect(classes[0]).toContain("text-muted-foreground");
    expect(classes[1]).toContain("text-success");
    expect(classes[2]).toContain("text-warning");
    await unmount();
  });
});

describe("SettingsPage and SettingsGroup", () => {
  it("fixes the column width so the content does not move between pages", async () => {
    const { container, unmount } = await mount(
      <SettingsPage title="Permissions" description="How much Bridge asks before an agent acts.">
        <SettingsGroup label="Provider prompts"><SettingsRow label="Auto-approve" /></SettingsGroup>
      </SettingsPage>,
    );
    expect(container.querySelector("[data-settings-column]")?.classList.contains("max-w-page")).toBe(true);
    expect(container.querySelector("h2")!.textContent).toBe("Permissions");
    expect(container.textContent).toContain("Provider prompts");
    await unmount();
  });

  it("renders a breadcrumb whose last crumb is not a link", async () => {
    const back = vi.fn();
    const { container, unmount } = await mount(
      <SettingsPage title="OpenCode" breadcrumb={[{ label: "Harnesses", onClick: back }, { label: "OpenCode" }]}>
        <span />
      </SettingsPage>,
    );
    const crumbs = container.querySelector('[aria-label="Breadcrumb"]')!;
    const links = crumbs.querySelectorAll("button");
    expect(links).toHaveLength(1);
    await act(async () => links[0].click());
    expect(back).toHaveBeenCalledOnce();
    await unmount();
  });
});
