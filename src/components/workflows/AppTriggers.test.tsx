// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { WorkflowEditor, composeEventTrigger } from "./WorkflowEditor";
import { eventFilters, triggerSummary } from "./format";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const settle = (ms = 0) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim().includes(label));
const field = <T extends HTMLElement>(label: string) => document.querySelector<T>(`[aria-label="${label}"]`);
function change(element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}

describe("app triggers", () => {
  it("say each app's event as a person would", () => {
    expect(triggerSummary({ kind: "event", event: "gmail_message_received", from: "Alice" })).toBe("When a new email arrives from Alice");
    expect(triggerSummary({ kind: "event", event: "linear_issue_assigned", channel: "Platform" })).toBe("When an issue is assigned to me in Linear (team Platform)");
    expect(triggerSummary({ kind: "event", event: "notion_page_created", channel: "Roadmap", from: "Priya" })).toBe("When a page is added to Roadmap in Notion by Priya");
    expect(triggerSummary({ kind: "event", event: "notion_page_created" })).toBe("When a page is added to a Notion database");
  });

  it("keep only the filters each event takes", () => {
    const fields = { repo: "acme/app", channel: "Roadmap", from: "Priya", collect: false, batchCron: null, timezone: "UTC" };
    expect(composeEventTrigger({ ...fields, event: "gmail_message_received" })).toMatchObject({ repo: null, channel: null, from: "Priya" });
    expect(composeEventTrigger({ ...fields, event: "linear_issue_assigned" })).toMatchObject({ repo: null, channel: "Roadmap", from: null });
    expect(composeEventTrigger({ ...fields, event: "notion_page_created" })).toMatchObject({ repo: null, channel: "Roadmap", from: "Priya" });
    expect(eventFilters("notion_page_created").channel?.required).toBe(true);
    expect(eventFilters("gmail_message_received").channel).toBeUndefined();
  });

  it("offer every app in the picker and ask for a Notion database before saving", async () => {
    const options = await bridgeApi.workflowOptions();
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={() => {}}/>));
    change(field<HTMLInputElement>("Workflow name")!, "Roadmap pages");
    change(field<HTMLTextAreaElement>("Workflow instructions")!, "Summarise the new page.");
    change(field<HTMLSelectElement>("Workflow schedule")!, "event");
    await settle(0);
    const groups = [...document.querySelectorAll('[aria-label="Event"] optgroup')].map(group => group.getAttribute("label"));
    expect(groups).toEqual(["Slack", "GitHub", "Gmail", "Linear", "Notion"]);

    change(field<HTMLSelectElement>("Event")!, "gmail_message_received");
    await settle(500);
    expect(field("Channel filter")).toBeNull();
    expect(field("Repository")).toBeNull();
    expect(field("Sender filter")?.closest("label")?.textContent).toContain("From (optional)");
    expect(document.body.textContent).toContain("Bridge has not read Gmail yet");

    change(field<HTMLSelectElement>("Event")!, "linear_issue_assigned");
    await settle(0);
    expect(field("Channel filter")?.closest("label")?.textContent).toContain("Team (optional)");
    expect(field("Sender filter")).toBeNull();

    change(field<HTMLSelectElement>("Event")!, "notion_page_created");
    await settle(500);
    expect(field("Channel filter")?.closest("label")?.textContent).toMatch(/^Database(?! \(optional\))/);
    expect(button("Approve and save")!.disabled).toBe(true);
    expect(document.body.textContent).toContain("Choose the Notion database to watch");
    change(field<HTMLInputElement>("Channel filter")!, "Roadmap");
    await settle(500);
    expect(button("Approve and save")!.disabled).toBe(false);
  });
});
