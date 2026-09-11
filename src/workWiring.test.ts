import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// How Work is wired into the app shell.
//
// This repo does not mount the whole App in a test — it needs a Tauri host — so the
// wiring is checked where it is declared instead. That is the same approach
// `designSystem.test.ts` takes, and it catches the regressions that matter here:
// the board being loaded eagerly, or the Work path growing a call that starts
// something.

const APP = readFileSync(join(__dirname, "App.tsx"), "utf8");
const SERVER_STATE = readFileSync(join(__dirname, "serverState.ts"), "utf8");
const SIDEBAR = readFileSync(join(__dirname, "components", "BridgeSidebar.tsx"), "utf8");

/** One `useCallback` body from App.tsx, ending at its dependency array.
 *
 * Bounded precisely rather than by a blank line: an over-wide slice would pick up
 * the next declaration and make these assertions pass or fail for the wrong reason. */
function declaration(name: string): string {
  const start = APP.indexOf(name);
  expect(start, `${name} is declared in App.tsx`).toBeGreaterThan(-1);
  const rest = APP.slice(start);
  const end = rest.indexOf("\n  }, [");
  expect(end, `${name} is a useCallback closed by a dependency array`).toBeGreaterThan(-1);
  return rest.slice(0, end);
}

describe("the Work view is lazy", () => {
  it("is loaded with lazy() and rendered inside Suspense", () => {
    // The board is a screen most sessions never open. Bundling it into the initial
    // chunk would make every cold start pay for it.
    expect(APP).toMatch(/const WorkView = lazy\(\(\) => import\("\.\/components\/WorkView"\)/);
    // The render branch, not the title strip — both test the same view value, and
    // only one of them renders the board.
    const branch = APP.slice(APP.indexOf('view === "work" ? <'));
    expect(branch.slice(0, 160)).toContain("<Suspense");
    expect(branch.slice(0, 160)).toContain("PanelLoading");
    expect(branch.slice(0, 160)).toContain("<WorkView");
  });

  it("imports the board's types without importing the board", () => {
    // A value import from WorkView would defeat the lazy boundary. The outcome type
    // is imported as a type, which is erased.
    expect(APP).toContain('import type { WorkActionOutcome } from "./components/WorkView";');
    expect(APP).not.toMatch(/^import \{[^}]*WorkView[^}]*\} from "\.\/components\/WorkView"/m);
  });
});

describe("opening Work starts nothing", () => {
  it("declares one lazy query that reads the board and nothing else", () => {
    // The acceptance criterion this slice rests on: opening Work must not select or
    // create a session, or start a model, connector, git command, or request. The
    // disabled query is the only read path.
    const query = SERVER_STATE.slice(SERVER_STATE.indexOf("const workBoard"));
    expect(query.match(/bridgeApi\.\w+/g)).toEqual(["bridgeApi.workBoard"]);
    expect(query).toContain("queryKey: queryKeys.workBoard");
    expect(query).toContain('enabled: query => !!followedRunId || query.state.data?.suggestions.state === "running"');
    expect(query).not.toMatch(/createSession|startTurn|setSelectedSessionId|openSession/);
  });

  it("opens the view without selecting a session", () => {
    const open = declaration("const openWorkBoard");
    expect(open).toContain('setView("work")');
    expect(open).toContain("refetchWorkBoard()");
    expect(open).not.toContain("setSelectedSessionId");
    expect(open).not.toContain("openSession");
  });

  it("navigates for a decision and calls only for Bridge's own work", () => {
    // A board button must never answer an approval on the user's behalf; it takes
    // them to where the decision is made. A fast-forward and a re-measure are
    // Bridge's own work and do call.
    const run = declaration("const runWorkAction");
    const [navigation, work] = [
      run.slice(run.indexOf("reviewCompletionCheck"), run.indexOf("refreshWorkspaceBase")),
      run.slice(run.indexOf('case "refreshWorkspaceBase"')),
    ];
    expect(navigation).toContain("openSession");
    expect(navigation).not.toContain("bridgeApi.");
    expect(work).toContain("bridgeApi.refreshWorkspaceBase");
    expect(work).toContain("bridgeApi.workspaceBaseDivergence");
  });
});

describe("the shell knows about Work", () => {
  it("names the view in the title strip", () => {
    expect(APP).toContain('view === "work" ? "Work"');
  });

  it("keeps the Needs-you count off the rail; Work board is wired but hidden from the nav (#458)", () => {
    expect(APP).toContain("onOpenWorkBoard={openWorkBoard}");
    expect(APP).not.toContain("workBoardActive");
    expect(APP).not.toContain("workNeedsYouCount");
    expect(APP).not.toContain('import { needsYouCount } from "./components/workFacts";');
    expect(SIDEBAR).toContain("onOpenWorkBoard");
    expect(SIDEBAR).not.toContain("Work board");
    expect(SIDEBAR).not.toContain("Needs you");
  });

  it("does not revive the Work/Code scope switch", () => {
    const footer = SIDEBAR.slice(SIDEBAR.indexOf("onOpenProjects}"));
    expect(footer).not.toContain('aria-label="Work"');
    expect(SIDEBAR).not.toContain('label="Work board"');
  });

  it("counts what needs you on the board itself, not a rail badge", () => {
    expect(APP).not.toContain('import { needsYouCount } from "./components/workFacts";');
    expect(SIDEBAR).not.toContain("workNeedsYouCount");
  });
});

describe("cached reads", () => {
  it("delegates request deduplication and stale response handling to TanStack Query", () => {
    expect(APP).toContain("QueryClientProvider");
    expect(APP).toContain("refetchWorkBoard");
    expect(APP).not.toContain("workReadGeneration");
    expect(APP).not.toContain("workBoardRef");
  });

  it("never replaces a board on screen with a read failure", () => {
    expect(APP).not.toContain("setWorkBoard");
    expect(APP).toContain("const workError = workBoard === undefined ? workQueryError : undefined;");
    expect(APP).toContain("const workRefreshError = workBoard === undefined ? undefined : workBriefingError ?? workQueryError;");
  });

  it("hands the view the refresh failure separately from the fatal one", () => {
    expect(APP).toContain("refreshError={workRefreshError}");
    expect(APP).toContain("error={workError}");
  });
});

describe("briefing runs are hidden from every surface", () => {
  it("filters state.sessions once, and nothing downstream reads the raw list", () => {
    // Found in review: the previous version of this test string-matched
    // `!isHiddenSession(s)` anywhere in the file and never checked which props got the
    // filtered list — so Agent Fleet was handed `state.sessions` while the comment
    // above claimed otherwise. Checking the props is the assertion that has teeth.
    expect(APP).toContain("const visibleSessions = useMemo(() => state.sessions.filter(s => !isHiddenSession(s))");
    // Every session-list prop must come from the filtered list. `state.sessions` may
    // appear only where it is being filtered.
    const rawUses = [...APP.matchAll(/sessions=\{state\.sessions\}/g)];
    expect(rawUses).toHaveLength(0);
  });

  it("hands Agent Fleet the workspace list for independent terminals", () => {
    // The surface the earlier miss actually affected: a briefing run is not idle or
    // done, so it would have appeared on the grid — and focusing it then failed,
    // because session resolution did apply the predicate.
    const start = APP.indexOf("<AgentFleet");
    const missionControl = APP.slice(start, APP.indexOf("</Suspense>", start));
    expect(missionControl.slice(0, 400)).toContain("workspaces={state.workspaces}");
    expect(missionControl.slice(0, 400)).not.toContain("sessions=");
  });

  it("builds the rail's list from the filtered one too", () => {
    const topSessions = APP.slice(APP.indexOf("const topSessions"));
    expect(topSessions.slice(0, 200)).toContain("visibleSessions.filter");
  });

  it("cannot be reached by selecting one directly either", () => {
    const resolved = APP.slice(APP.indexOf("const session = state.sessions.find"));
    expect(resolved.slice(0, 160)).toContain("!isHiddenSession(s)");
  });
});

it("opens Work from settings without restoring the sidebar row", () => {
  expect(APP).toContain("<SettingsScreen onOpenWorkBoard={openWorkBoard}");
  const settings = readFileSync(join(__dirname, "components", "SettingsScreen.tsx"), "utf8");
  expect(settings).toContain("onOpenBoard={onOpenWorkBoard}");
});
