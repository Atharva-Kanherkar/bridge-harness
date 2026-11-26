// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatOverviewModel, TurnHistoryEntry } from "../chatOverview";
import { dockPaneChord } from "../dockLayout";
import { formatShortcut } from "@/lib/platform";
import { bridgeApi } from "../api";
import type { Session, Workspace } from "../types";
import { ChatOverview, ChatOverviewView } from "./ChatOverview";
import { WorkPane } from "./WorkPane";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const turn = (over: Partial<TurnHistoryEntry>): TurnHistoryEntry => ({
  turn: 1, key: `t-${over.turn ?? 1}`, title: "Read policy engine", live: false, changes: null, ...over,
});

const full: ChatOverviewModel = {
  headline: "Changes to review",
  rows: [
    { kind: "where", path: "/Users/you/bridge/.worktrees/retry-budget", display: "…/bridge/.worktrees/retry-budget", detail: "bridge-harness" },
    { kind: "branch", name: "bridge/retry-budget", detail: "2 ahead of origin/main" },
    { kind: "changes", files: 4, additions: 86, deletions: 12 },
    { kind: "checks", tone: "passed", label: "Verified", detail: "2 of 2 required checks passed" },
  ],
  turns: [
    turn({ turn: 2, title: "Add retry budget", durationMs: 34_000, entryId: "entry-2", changes: { files: 4, additions: 86, deletions: 12 } }),
    turn({ turn: 1, title: "Read policy engine", durationMs: 12_000, entryId: "entry-1" }),
  ],
};

const rowKinds = () => [...host.querySelectorAll<HTMLElement>("[data-overview-row]")].map(row => row.dataset.overviewRow);
const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);

describe("ChatOverviewView", () => {
  it("renders each row it has data for, with its label and detail", () => {
    act(() => root.render(<ChatOverviewView model={full} harness="codex" />));
    expect(host.textContent).toContain("This chat");
    expect(host.querySelector("h2")!.textContent).toBe("Changes to review");
    expect(rowKinds()).toEqual(["where", "branch", "changes", "checks"]);
    const changes = host.querySelector("[data-overview-row=changes]")!;
    expect(changes.textContent).toContain("4 files");
    expect(changes.textContent).toContain("+86");
    expect(changes.textContent).toContain("−12");
    expect(changes.textContent).toContain("uncommitted");
    expect(host.querySelector("[data-overview-row=branch]")!.textContent).toContain("2 ahead of origin/main");
    expect(host.querySelector("[data-overview-row=where]")!.querySelector("[data-tip]")!.getAttribute("data-tip")).toBe("/Users/you/bridge/.worktrees/retry-budget");
  });

  it("draws nothing for absent facts and offers no Commit or Open PR", () => {
    act(() => root.render(<ChatOverviewView model={{ rows: [{ kind: "branch", name: "main" }], turns: [] }} />));
    expect(rowKinds()).toEqual(["branch"]);
    expect(host.querySelector("h2")).toBeNull();
    expect(host.textContent).not.toContain("Turn history");
    expect(host.textContent).not.toMatch(/Commit|Open PR/);
  });

  it("opens the Changes pane from Review, with the pane's own chord", () => {
    const onReview = vi.fn();
    act(() => root.render(<ChatOverviewView model={full} onReview={onReview} />));
    const review = button("Review 4 files")!;
    expect(review.textContent).toContain(formatShortcut({ meta: true, alt: true }, dockPaneChord("changes")!));
    act(() => review.click());
    expect(onReview).toHaveBeenCalledTimes(1);
  });

  it("opens the Code pane from Where, and hides both actions without a handler", () => {
    const onOpenCode = vi.fn();
    act(() => root.render(<ChatOverviewView model={full} onOpenCode={onOpenCode} />));
    act(() => button("Open in the Code pane")!.click());
    expect(onOpenCode).toHaveBeenCalledTimes(1);
    expect(button("Review 4 files")).toBeNull();
    act(() => root.render(<ChatOverviewView model={full} />));
    expect(button("Open in the Code pane")).toBeNull();
  });

  it("offers no Review on a clean tree", () => {
    act(() => root.render(<ChatOverviewView model={{ rows: [{ kind: "changes", files: 0, additions: 0, deletions: 0 }], turns: [] }} onReview={() => undefined} />));
    expect(host.textContent).toContain("No uncommitted changes");
    expect(host.querySelector("button")).toBeNull();
  });

  it("lists turns with duration and change count, and reveals a turn's prompt", () => {
    const onOpenTurn = vi.fn();
    act(() => root.render(<ChatOverviewView model={full} harness="codex" onOpenTurn={onOpenTurn} />));
    const turns = [...host.querySelectorAll<HTMLElement>("[data-overview-turn]")];
    expect(turns.map(row => row.dataset.overviewTurn)).toEqual(["2", "1"]);
    expect(turns[0].textContent).toContain("Add retry budget");
    expect(turns[0].textContent).toContain("34s");
    expect(turns[0].textContent).toContain("4 files");
    expect(turns[1].textContent).toContain("no changes");
    expect(turns[0].querySelector("[data-harness]")!.getAttribute("data-harness")).toBe("codex");
    act(() => button("Show turn: Read policy engine")!.click());
    expect(onOpenTurn).toHaveBeenCalledWith("entry-1");
  });

  it("marks the running turn Working without a duration or a count", () => {
    act(() => root.render(<ChatOverviewView model={{ rows: [], turns: [turn({ turn: 3, title: "Run the suite", live: true })] }} />));
    const row = host.querySelector("[data-overview-turn='3']")!;
    expect(row.textContent).toContain("Working");
    expect(row.textContent).not.toContain("no changes");
    // Without a reveal handler the row is not a button.
    expect(row.querySelector("button")).toBeNull();
  });

  it("folds a long history behind Show more", () => {
    const turns = Array.from({ length: 8 }, (_, index) => turn({ turn: 8 - index, key: `t-${index}`, title: `Turn ${8 - index}` }));
    act(() => root.render(<ChatOverviewView model={{ rows: [], turns }} />));
    expect(host.querySelectorAll("[data-overview-turn]").length).toBe(5);
    const more = [...host.querySelectorAll("button")].find(element => element.textContent === "Show 3 more")!;
    act(() => more.click());
    expect(host.querySelectorAll("[data-overview-turn]").length).toBe(8);
    expect([...host.querySelectorAll("button")].some(element => element.textContent === "Show fewer")).toBe(true);
  });
});

const session = { id: "s-1", harness: "codex", cwd: "/Users/you/bridge/Kyoto", status: "ready" } as Session;
const workspace = { id: "w-1", title: "Kyoto", branch: "bridge/session-supervisor", path: "/Users/you/bridge/Kyoto", dirtyFiles: 4, additions: 284, deletions: 31 } as Workspace;

describe("ChatOverview", () => {
  it("reads ahead/behind without a fetch, only while visible, and routes Review to the Changes pane", async () => {
    const divergence = vi.spyOn(bridgeApi, "workspaceBaseDivergence").mockResolvedValue({
      ahead: 2, behind: 0, baseRef: "origin/main", branch: "bridge/session-supervisor", dirty: true, fetchAttempted: false, fetched: false, unavailableReason: null,
    });
    const onOpenPane = vi.fn();
    const render = (visible: boolean) => root.render(<ChatOverview session={session} workspace={workspace} projectName="Bridge" events={[]} turnActive={false} visible={visible} onOpenPane={onOpenPane} />);
    act(() => render(false));
    expect(divergence).not.toHaveBeenCalled();
    await act(async () => render(true));
    expect(divergence).toHaveBeenCalledWith("s-1", false);
    expect(host.querySelector("[data-overview-row=branch]")!.textContent).toContain("2 ahead of origin/main");
    expect(host.querySelector("[data-overview-row=where]")!.textContent).toContain("Bridge");
    act(() => button("Review 4 files")!.click());
    expect(onOpenPane).toHaveBeenCalledWith("changes");
    act(() => button("Open in the Code pane")!.click());
    expect(onOpenPane).toHaveBeenCalledWith("code");
  });

  it("asks nothing of git and offers no pane actions for a chat without a repository", async () => {
    const divergence = vi.spyOn(bridgeApi, "workspaceBaseDivergence");
    await act(async () => root.render(<ChatOverview session={{ ...session, cwd: null }} events={[]} turnActive={false} visible onOpenPane={() => undefined} />));
    expect(divergence).not.toHaveBeenCalled();
    expect(host.querySelectorAll("[data-overview-row]").length).toBe(0);
    expect(host.querySelector("button")).toBeNull();
  });
});

describe("WorkPane overview slot", () => {
  it("draws the overview above the pull requests", () => {
    act(() => root.render(<WorkPane overview={<ChatOverviewView model={full} />} prs={[]} tasks={[]} refreshing={false} onAttach={async () => undefined} onRetry={() => undefined} />));
    const pane = host.querySelector("[data-work-pane]")!;
    expect(pane.firstElementChild!.hasAttribute("data-chat-overview")).toBe(true);
    expect(pane.textContent).toContain("Attach a pull request");
    // A working chat's overview must not sit above "Nothing running".
    expect(pane.textContent).not.toContain("Nothing running");
    expect(pane.textContent).toContain("Pull requests this chat opens");
  });
});
