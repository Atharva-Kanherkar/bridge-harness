// @vitest-environment jsdom
// The Time tab's breakdown cards, rules 13 to 17 of
// testing/feat-screen-time-breakdown.md.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ScreenTimeHour, ScreenTimeParams, ScreenTimeProject, UsageScreenTimeResult } from "../types";
import { clearScreenTimeCacheForTests, UsageScreenTime } from "./UsageScreenTime";
import { HarnessesCard, hourName, HoursCard, PROJECT_ROWS, ProjectsCard, PullRequestsCard } from "./UsageScreenTimeBreakdown";

let container: HTMLDivElement;
let root: Root;

const HOUR = 3_600;
const EMPTY_PRS = { opened: 0, merged: 0, open: 0, closed: 0, repos: [], recent: [] };

function project(name: string, human: number, agent: number, overrides: Partial<ScreenTimeProject> = {}): ScreenTimeProject {
  return { projectId: `p-${name}`, name, humanSeconds: human, agentSeconds: agent, chats: 2, pullRequests: 1, ...overrides };
}

function hours(fill: (hour: number) => [number, number] = () => [0, 0]): ScreenTimeHour[] {
  return Array.from({ length: 24 }, (_, hour) => { const [human, agent] = fill(hour); return { hour, humanSeconds: human, agentSeconds: agent }; });
}

const settle = () => act(async () => { for (let i = 0; i < 5; i += 1) await Promise.resolve(); });
const text = () => container.textContent ?? "";
const render = (node: React.ReactNode) => act(async () => { root.render(node); });
const section = (label: string) => container.querySelector(`section[aria-label="${label}"]`);

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  clearScreenTimeCacheForTests();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("ProjectsCard", () => {
  it("lists each project's time, chats and pull requests, with No project muted", async () => {
    await render(<ProjectsCard projects={[project("harness", 2 * HOUR, 30 * HOUR, { chats: 14, pullRequests: 9 }), project("No project", 0, HOUR, { projectId: null, chats: 3, pullRequests: 0 })]} />);
    const rows = [...container.querySelectorAll("tbody tr")].map(row => [...row.children].map(cell => cell.textContent));
    expect(rows).toEqual([["harness", "2h 00m", "30h 00m", "14", "9"], ["No project", "0m", "1h 00m", "3", "0"]]);
    expect(container.querySelector('[data-project="none"] th span')?.className).toContain("text-muted-foreground");
    expect(text()).toContain("2 projects");
    expect(text()).toContain("from this update on");
  });

  it("shows six rows until Show all", async () => {
    const many = Array.from({ length: PROJECT_ROWS + 3 }, (_, index) => project(`p${index}`, HOUR, (20 - index) * HOUR));
    await render(<ProjectsCard projects={many} />);
    expect(container.querySelectorAll("tbody tr")).toHaveLength(PROJECT_ROWS);
    const toggle = [...container.querySelectorAll("button")].find(button => button.textContent === `Show all ${PROJECT_ROWS + 3}`)!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await act(async () => { toggle.click(); });
    expect(container.querySelectorAll("tbody tr")).toHaveLength(PROJECT_ROWS + 3);
    expect(toggle.textContent).toBe("Show fewer");
  });

  it("says so when the range has no project time", async () => {
    await render(<ProjectsCard projects={[]} />);
    expect(text()).toContain("No project time in this range.");
    expect(container.querySelector("table")).toBeNull();
  });
});

describe("PullRequestsCard", () => {
  it("counts by state, splits by repository and links the latest", async () => {
    await render(<PullRequestsCard pullRequests={{
      opened: 12, merged: 9, open: 2, closed: 1,
      repos: [{ repo: "me/bridge", opened: 10, merged: 8 }, { repo: "me/kairo", opened: 2, merged: 1 }],
      recent: [
        { repo: "me/bridge", number: 42, title: "fix: count chats once", url: "https://github.com/me/bridge/pull/42", state: "merged", openedDay: "2026-11-25" },
        { repo: "me/kairo", number: 7, title: "feat: page the log", url: "https://github.com/me/kairo/pull/7", state: "open", openedDay: "2026-11-24" },
      ],
    }} />);
    expect(container.querySelector('[data-testid="screen-time-prs-opened"]')?.textContent).toBe("12");
    expect(text()).toContain("opened · 9 merged · 2 open · 1 closed");
    const repos = [...container.querySelectorAll('[aria-label="By repository"] li')].map(item => item.textContent);
    expect(repos).toEqual(["me/bridge10 PRs · 8 merged", "me/kairo2 PRs · 1 merged"]);
    const links = [...container.querySelectorAll<HTMLAnchorElement>('[aria-label="Latest pull requests"] a')];
    expect(links.map(link => link.getAttribute("href"))).toEqual(["https://github.com/me/bridge/pull/42", "https://github.com/me/kairo/pull/7"]);
    expect(links[0].getAttribute("aria-label")).toBe("fix: count chats once, me/bridge #42, Merged");
    expect(links[0].textContent).toContain("bridge #42 · Nov 25");
    expect(links[0].hasAttribute("data-system-browser")).toBe(true);
    expect(links[0].querySelector("svg")?.getAttribute("class")).toContain("text-merged");
  });

  it("says so when no pull request was opened", async () => {
    await render(<PullRequestsCard pullRequests={EMPTY_PRS} />);
    expect(text()).toContain("No pull requests opened from Bridge in this range.");
  });
});

describe("HoursCard", () => {
  it("names your busiest hour, else the agents', and reads one hour on hover", async () => {
    await render(<HoursCard hours={hours(hour => [hour === 14 ? 2 * HOUR : hour === 9 ? HOUR : 0, hour === 3 ? 5 * HOUR : HOUR])} />);
    const status = () => section("When you work")?.querySelector('[role="status"]')?.textContent ?? "";
    expect(container.querySelectorAll("[data-hour]")).toHaveLength(24);
    expect(status()).toBe("Your busiest hour: 2 pm");
    await act(async () => { container.querySelector('[data-hour="9"]')!.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })); });
    expect(status()).toBe("9 am: you 1h 00m · agents 1h 00m");
    expect(text()).toContain("12a6a12p6p");

    await render(<HoursCard key="agents-only" hours={hours(hour => [0, hour === 23 ? HOUR : 0])} />);
    expect(status()).toBe("Agents' busiest hour: 11 pm");
    expect(hourName(0)).toBe("12 am");
    expect(hourName(12)).toBe("12 pm");
  });

  it("says so when the range is empty", async () => {
    await render(<HoursCard hours={hours()} />);
    expect(text()).toContain("No time recorded in this range.");
  });
});

describe("HarnessesCard", () => {
  it("shows each harness's agent time, chats and share", async () => {
    await render(<HarnessesCard harnesses={[{ harness: "claude", agentSeconds: 30 * HOUR, chats: 12 }, { harness: "codex", agentSeconds: 10 * HOUR, chats: 1 }]} />);
    const rows = [...container.querySelectorAll("li[data-harness]")].map(row => row.textContent);
    expect(rows[0]).toMatch(/Claude.*12 chats30h 00m75%$/);
    expect(rows[1]).toMatch(/Codex.*1 chat10h 00m25%$/);
    expect(container.querySelector('li[data-harness="claude"] rect:last-child')?.getAttribute("width")).toBe("75");
  });

  it("says so with no agent time", async () => {
    await render(<HarnessesCard harnesses={[]} />);
    expect(text()).toContain("No agent time in this range.");
  });
});

describe("UsageScreenTime breakdown", () => {
  it("draws all four cards from the screen_time result", async () => {
    vi.spyOn(bridgeApi, "screenTime").mockImplementation(async (params: ScreenTimeParams): Promise<UsageScreenTimeResult> => ({
      sinceDay: params.sinceDay,
      untilDay: params.untilDay,
      timeZone: params.timeZone ?? "UTC",
      days: [{ day: params.untilDay, humanSeconds: HOUR, agentSeconds: 4 * HOUR }],
      allTime: { humanSeconds: HOUR, agentSeconds: 4 * HOUR, humanSinceDay: params.untilDay, agentSinceDay: params.untilDay },
      agentHistory: "complete",
      projects: [project("harness", HOUR, 4 * HOUR)],
      pullRequests: { ...EMPTY_PRS, opened: 1, merged: 1, repos: [{ repo: "me/bridge", opened: 1, merged: 1 }] },
      hours: hours(hour => hour === 10 ? [HOUR, 4 * HOUR] : [0, 0]),
      harnesses: [{ harness: "claude", agentSeconds: 4 * HOUR, chats: 2 }],
    }));
    await render(<UsageScreenTime onError={() => undefined} />);
    await settle();
    for (const label of ["Daily time", "Projects", "Pull requests", "When you work", "Agents by harness", "Your total time"]) expect(section(label)).not.toBeNull();
    const order = [...container.querySelectorAll("section[aria-label]")].map(node => node.getAttribute("aria-label"));
    expect(order).toEqual(["Summary", "Daily time", "Projects", "Pull requests", "When you work", "Agents by harness", "Your total time"]);
    expect(section("Projects")?.textContent).toContain("harness");
    expect(section("When you work")?.textContent).toContain("Your busiest hour: 10 am");
  });
});
