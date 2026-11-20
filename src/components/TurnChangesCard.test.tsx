// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatShortcut } from "@/lib/platform";
import { dockPaneChord } from "../dockLayout";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session } from "../types";
import { AgentConversation } from "./AgentConversation";

const session: Session = {
  id: "s", workspaceId: "w", harness: "codex", label: "Chat", status: "idle", startedAt: "now", endedAt: null,
  contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna",
  restorationMode: "fresh", continuationFidelity: "native", kind: "chat",
} as Session;

const at = (seconds: number) => new Date(Date.UTC(2026, 10, 20, 18, 17, seconds)).toISOString();
let id = 0;
const event = (kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => {
  id += 1;
  return {
    id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
    role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {}, createdAt: at(0), ...overrides,
  };
};
const MODIFY = "--- a/src/policy.rs\n+++ b/src/policy.rs\n@@ -1,2 +1,3 @@\n fn a() {}\n-fn b() {}\n+fn b() { 1 }\n+fn c() {}";
const CREATE = "--- /dev/null\n+++ b/testing/retry.md\n@@ -0,0 +1,2 @@\n+# Retry\n+budget";

/** A settled turn: a check, two edits of one file, a new file, then the reply. */
const editingTurn = (): AgentEvent[] => [
  event("message.completed", { role: "user", text: "Add a retry budget.", createdAt: at(0) }),
  event("command.completed", { title: "cargo test", createdAt: at(0), data: { type: "commandExecution", command: "cargo test", exitCode: 0, durationMs: 4000 } }),
  event("file_change.completed", { title: "policy.rs", createdAt: at(10), data: { path: "src/policy.rs", additions: 2, deletions: 1, patch: MODIFY, durationMs: 1000 } }),
  event("file_change.completed", { title: "policy.rs", createdAt: at(20), data: { path: "src/policy.rs", additions: 3, deletions: 0 } }),
  event("file_change.completed", { title: "retry.md", createdAt: at(30), data: { path: "testing/retry.md", patch: CREATE, durationMs: 1000 } }),
  event("message.completed", { role: "assistant", text: "Added the retry budget.", createdAt: at(34) }),
];

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

function mount(props: Partial<React.ComponentProps<typeof AgentConversation>> = {}) {
  act(() => {
    root.render(<AgentConversation session={session} events={editingTurn()} onResolve={() => undefined} {...props} />);
  });
}

const card = () => host.querySelector<HTMLElement>("[data-turn-changes]");
const rows = () => [...host.querySelectorAll<HTMLElement>("[data-turn-file]")];

describe("the turn changes card", () => {
  it("closes a settled turn with its totals and a folder tree of what it changed", () => {
    mount();
    const section = card()!;
    expect(section).not.toBeNull();
    const header = section.querySelector("header")!.textContent;
    // A file edited twice is one row; the totals sum every edit.
    expect(header).toContain("2 files changed");
    expect(header).toContain("+7");
    expect(header).toContain("−1");
    expect(header).toContain("in this turn");
    const folders = [...section.querySelectorAll('[role="group"]')].map(group => group.getAttribute("aria-label"));
    expect(folders).toEqual(["src/", "testing/"]);
    expect(rows().map(row => [row.dataset.turnFile, row.textContent])).toEqual([
      ["src/policy.rs", "Mpolicy.rs+5−1"],
      ["testing/retry.md", "Aretry.md+2−0"],
    ]);
    // It sits inside the reply, ahead of the reply's actions.
    const reply = section.closest(".group")!;
    expect(reply.textContent).toContain("Added the retry budget.");
    expect(reply.querySelector("[data-turn-footer]")).not.toBeNull();
  });

  it("opens the Changes pane from Review, labelled with that pane's chord", () => {
    const onReviewChanges = vi.fn();
    mount({ onReviewChanges });
    const review = card()!.querySelector<HTMLButtonElement>("[data-review-changes]")!;
    expect(dockPaneChord("changes")).toBe("1");
    expect(review.querySelector("kbd")!.textContent).toBe(formatShortcut({ meta: true, alt: true }, "1"));
    act(() => review.click());
    expect(onReviewChanges).toHaveBeenCalledTimes(1);
  });

  it("has no Review button without an opener, or in a read-only history", () => {
    mount();
    expect(card()!.querySelector("[data-review-changes]")).toBeNull();
    mount({ readOnly: true, session: undefined, onReviewChanges: () => undefined });
    expect(card()).not.toBeNull();
    expect(card()!.querySelector("[data-review-changes]")).toBeNull();
  });

  it("shows nothing while the turn is still running", () => {
    mount({ working: true });
    expect(card()).toBeNull();
    expect(host.querySelector("[data-turn-line]")).toBeNull();
    mount({ working: false });
    expect(card()).not.toBeNull();
  });

  it("gives a turn without edits a caption but no card", () => {
    mount({ events: [
      event("message.completed", { role: "user", text: "What does the budget do?", createdAt: at(0) }),
      event("tool.completed", { title: "Read policy.rs", createdAt: at(1), data: { type: "readFile", path: "src/policy.rs" } }),
      event("message.completed", { role: "assistant", text: "It caps retries.", createdAt: at(5) }),
    ] });
    expect(card()).toBeNull();
    expect(host.querySelector("[data-turn-line]")?.textContent).toMatch(/GPT Luna$/);
  });

  it("expands every file's diff from Expand all, and folds a folder from its row", () => {
    mount();
    const expandAll = card()!.querySelector<HTMLButtonElement>("[data-expand-all]")!;
    expect(expandAll.textContent).toBe("Expand all");
    act(() => expandAll.click());
    expect(expandAll.textContent).toBe("Collapse all");
    expect(rows().map(row => row.querySelector("button")!.getAttribute("aria-expanded"))).toEqual(["true", "true"]);
    expect(rows()[1].textContent).toContain("budget");
    act(() => expandAll.click());
    expect(rows().map(row => row.querySelector("button")!.getAttribute("aria-expanded"))).toEqual(["false", "false"]);

    const folder = card()!.querySelector<HTMLButtonElement>('[aria-label="src/"] > button')!;
    act(() => folder.click());
    expect(folder.getAttribute("aria-expanded")).toBe("false");
    expect(rows().map(row => row.dataset.turnFile)).toEqual(["testing/retry.md"]);
  });

  it("ends a turn that has no reply under its last row", () => {
    mount({ events: editingTurn().slice(0, -1) });
    const end = host.querySelector("[data-turn-end]")!;
    expect(end).not.toBeNull();
    expect(end.querySelector("[data-turn-changes]")).not.toBeNull();
    expect(end.previousElementSibling?.matches("[data-activity-group]")).toBe(true);
  });
});

describe("the turn line", () => {
  it("reads time, the same duration as Worked for, and the model, beside the reply's actions", () => {
    mount({ onRemember: () => undefined });
    const line = host.querySelector("[data-turn-line]")!;
    const worked = [...host.querySelectorAll("[data-activity-group] button")].map(button => button.textContent ?? "").find(text => text.startsWith("Worked for"))!;
    expect(worked).toContain("Worked for 31s");
    const parts = line.textContent!.split(" · ");
    expect(parts).toHaveLength(3);
    expect(parts[0]).toMatch(/\d/);
    expect(parts.slice(1)).toEqual(["31s", "GPT Luna"]);
    const footer = line.closest("[data-turn-footer]")!;
    const actions = footer.querySelector("[data-reply-actions]")!;
    expect(actions.getAttribute("data-reply-actions")).toBe("latest");
    expect([...actions.querySelectorAll("button")].map(button => button.getAttribute("aria-label"))).toEqual(["Copy", "Remember this"]);
  });

  it("names the model a turn ran on, not the one the chat switched to later", () => {
    mount({
      session: { ...session, model: "opus", harness: "claude" },
      events: [
        ...editingTurn(),
        event("session.model_changed", { title: "Chat model changed", createdAt: at(40), data: { previousHarness: "codex", previousModel: "gpt-5.6-luna", harness: "claude", model: "opus" } }),
        event("message.completed", { role: "user", text: "Now explain it.", createdAt: at(50) }),
        event("message.completed", { role: "assistant", text: "It counts failures.", createdAt: at(55) }),
      ],
    });
    const lines = [...host.querySelectorAll("[data-turn-line]")].map(line => line.textContent!.split(" · ").at(-1));
    expect(lines).toEqual(["GPT Luna", "Opus"]);
  });

  it("keeps an older turn's actions hidden until hover, in the space before its caption", () => {
    mount({ events: [
      ...editingTurn(),
      event("message.completed", { role: "user", text: "Thanks.", createdAt: at(50) }),
      event("message.completed", { role: "assistant", text: "Any time.", createdAt: at(55) }),
    ] });
    const footers = [...host.querySelectorAll("[data-turn-footer]")];
    expect(footers).toHaveLength(2);
    const older = footers[0].querySelector("[data-reply-actions]")!;
    expect(older.getAttribute("data-reply-actions")).toBe("hover");
    expect(older.className).toContain("opacity-0");
    expect(older.className).not.toContain("absolute");
    expect(footers[1].querySelector("[data-reply-actions]")!.getAttribute("data-reply-actions")).toBe("latest");
  });
});

describe("the turn's worked-for summary", () => {
  it("names edited files, not edit calls, so it agrees with the changes card", () => {
    act(() => {
      root.render(<AgentConversation session={session} events={editingTurn()} onResolve={() => undefined} />);
    });
    // policy.rs is edited twice and retry.md once: two files, like the card.
    expect(host.textContent).toContain("edited 2 files");
    expect(host.querySelectorAll("[data-turn-file]")).toHaveLength(2);
  });
});
