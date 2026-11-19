// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationItem } from "../conversation";
import { QuestionCard, type ResolveQuestion } from "./QuestionCard";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const question = (data: Record<string, unknown>, overrides: Partial<ConversationItem> = {}): ConversationItem => ({
  key: "q", identity: "q", type: "question", eventId: 9, sequence: 9, turn: 1, title: "Database", text: "", status: "pending", data, ...overrides,
} as ConversationItem);

const claude = {
  requestMethod: "claude.askUserQuestion",
  questions: [
    { id: "0", question: "Which database?", header: "Database", multiSelect: false, allowOther: true,
      options: [{ label: "Postgres", description: "Relational, hosted" }, { label: "SQLite", description: "Embedded", preview: "app.db\n  users" }] },
    { id: "1", question: "Which features?", header: "Features", multiSelect: true, allowOther: true,
      options: [{ label: "Auth", description: "" }, { label: "Billing", description: "" }] },
  ],
};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const render = (item: ConversationItem, onResolve: ResolveQuestion, resolvedCopy = "Answered") =>
  act(() => root.render(<QuestionCard item={item} onResolve={onResolve} resolvedCopy={resolvedCopy} />));
const byRole = (role: string, name: string) => [...host.querySelectorAll<HTMLElement>(`[role="${role}"]`)].find(node => node.textContent?.includes(name) || node.getAttribute("aria-label") === name)!;
const button = (name: string) => [...host.querySelectorAll("button")].find(node => node.textContent?.includes(name))!;
const type = (input: HTMLInputElement | HTMLTextAreaElement, value: string) => act(() => {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});

describe("QuestionCard", () => {
  it("shows every question with descriptions and only sends once each is answered", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async () => undefined);
    render(question(claude), onResolve);
    expect(host.textContent).toContain("2 questions");
    expect(host.textContent).toContain("Relational, hosted");
    expect(host.textContent).toContain("0 of 2 answered");
    const send = button("Send answers");
    expect(send.disabled).toBe(true);

    act(() => byRole("radio", "SQLite").click());
    expect(byRole("radio", "SQLite").getAttribute("aria-checked")).toBe("true");
    // The chosen option's preview is shown.
    expect(host.querySelector("pre")?.textContent).toContain("app.db");
    expect(send.disabled).toBe(true);

    act(() => byRole("checkbox", "Auth").click());
    const other = host.querySelector<HTMLInputElement>('input[aria-label="Which features?: something else"]')!;
    type(other, "Search");
    expect(host.textContent).toContain("2 of 2 answered");
    expect(send.disabled).toBe(false);

    await act(async () => send.click());
    expect(onResolve).toHaveBeenCalledWith(9, "answer", { 0: ["SQLite"], 1: ["Auth", "Search"] });
    expect(host.textContent).toContain("Answer sent");
    expect(host.textContent).toContain("Auth, Search");
  });

  it("a typed Other replaces a single choice", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async () => undefined);
    render(question({ questions: [claude.questions[0]] }), onResolve);
    act(() => byRole("radio", "Postgres").click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="Which database?: something else"]')!, "DuckDB");
    expect(byRole("radio", "Postgres").getAttribute("aria-checked")).toBe("false");
    await act(async () => button("Send answer").click());
    expect(onResolve).toHaveBeenCalledWith(9, "answer", { 0: ["DuckDB"] });
  });

  it("picks with digit keys and sends with Cmd+Enter when there is one list", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async () => undefined);
    render(question({ questions: [claude.questions[0]] }), onResolve);
    const section = host.querySelector("section")!;
    act(() => { section.dispatchEvent(new KeyboardEvent("keydown", { key: "2", bubbles: true })); });
    expect(byRole("radio", "SQLite").getAttribute("aria-checked")).toBe("true");
    await act(async () => { section.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true })); });
    expect(onResolve).toHaveBeenCalledWith(9, "answer", { 0: ["SQLite"] });
  });

  it("answers free text, numbers and yes/no from an elicitation schema", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async () => undefined);
    render(question({ message: "Configure the deploy", requestedSchema: { required: ["name", "count"], properties: {
      name: { type: "string", title: "Name" },
      count: { type: "integer", title: "Replicas" },
      confirm: { type: "boolean", title: "Proceed?" },
    } } }), onResolve);
    expect(host.textContent).toContain("Configure the deploy");
    type(host.querySelector("textarea")!, "api");
    const number = host.querySelector<HTMLInputElement>('input[type="number"]')!;
    type(number, "1.5");
    expect(button("Send answer").disabled).toBe(true);
    type(number, "3");
    act(() => byRole("radio", "Yes").click());
    await act(async () => button("Send answer").click());
    expect(onResolve).toHaveBeenCalledWith(9, "answer", { name: ["api"], count: ["3"], confirm: ["true"] });
  });

  it("masks a secret and never shows it once sent", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async () => undefined);
    render(question({ questions: [{ id: "token", question: "Paste the token", isSecret: true }] }), onResolve);
    const input = host.querySelector<HTMLInputElement>('input[type="password"]')!;
    type(input, "sk-live-123");
    await act(async () => button("Send answer").click());
    expect(host.textContent).not.toContain("sk-live-123");
    expect(host.textContent).toContain("Hidden");
  });

  it("skips without answers, and keeps the card open when sending fails", async () => {
    const onResolve = vi.fn<ResolveQuestion>(async (_id, action) => { if (action === "answer") throw new Error("Structured adapter session is not running"); });
    render(question({ questions: [claude.questions[0]] }), onResolve);
    act(() => byRole("radio", "Postgres").click());
    await act(async () => button("Send answer").click());
    expect(host.textContent).toContain("Structured adapter session is not running");
    expect(button("Send answer")).toBeTruthy();
    await act(async () => button("Skip").click());
    expect(onResolve).toHaveBeenLastCalledWith(9, "decline", {});
  });

  it("shows the recorded answers after a reload", () => {
    render(question(claude, { status: "answered", data: { ...claude, submittedAnswers: { 0: ["Postgres"], 1: ["Billing"] } } }), vi.fn());
    expect(host.querySelector("button")).toBeNull();
    expect(host.textContent).toContain("Which database?");
    expect(host.textContent).toContain("Postgres");
    expect(host.textContent).toContain("Billing");
    expect(host.textContent).toContain("Answered");
  });
});
