// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Markdown } from "./Markdown";
import { RunInTerminalContext, type RunInTerminal } from "./RunInTerminal";

// Contract: testing/feat-run-in-terminal.md §Integration / Functional Tests.

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function render(text: string, run: RunInTerminal | null) {
  act(() => {
    root.render(<RunInTerminalContext.Provider value={run}><Markdown text={text} /></RunInTerminalContext.Provider>);
  });
}

const runButton = () => container.querySelector<HTMLButtonElement>("button[data-run-state]");
const copyButtons = () => [...container.querySelectorAll("button")].filter(button => button.getAttribute("aria-label") === "Copy");

describe("Run in terminal on chat code blocks", () => {
  it("offers Run on a shell block, only where a terminal is provided", () => {
    const run = vi.fn<RunInTerminal>();
    render("```bash\nbun test\n```", run);
    expect(runButton()?.getAttribute("aria-label")).toBe("Run in terminal");
    expect(runButton()?.textContent).toBe("Run");
    expect(copyButtons()).toHaveLength(1);

    render("```ts\nconst a = 1;\n```", run);
    expect(runButton()).toBeNull();

    render("```bash\nbun test\n```", null);
    expect(runButton()).toBeNull();
    expect(copyButtons()).toHaveLength(1);
  });

  it("never offers Run on a fence that is still streaming", () => {
    render("```bash\nrm -rf buil", vi.fn<RunInTerminal>());
    expect(runButton()).toBeNull();
    expect(copyButtons()).toHaveLength(1);
  });

  it("sends the extracted command and walks through running, sent and idle", async () => {
    let resolve: (value: { shell: string }) => void = () => undefined;
    const run = vi.fn<RunInTerminal>(() => new Promise(done => { resolve = done; }));
    render("```console\n$ bun install\nadded 12 packages\n```", run);

    act(() => runButton()!.click());
    expect(run).toHaveBeenCalledWith("bun install");
    expect(runButton()?.textContent).toBe("Run");
    expect(runButton()?.getAttribute("aria-busy")).toBe("true");
    expect(runButton()?.disabled).toBe(true);

    act(() => runButton()!.click());
    expect(run).toHaveBeenCalledTimes(1);

    await act(async () => { resolve({ shell: "shell 1" }); });
    expect(runButton()?.textContent).toBe("Sent to shell 1");
    expect(runButton()?.getAttribute("data-run-state")).toBe("sent");

    act(() => { vi.advanceTimersByTime(2_000); });
    expect(runButton()?.textContent).toBe("Run");
  });

  it("says when a run fails and why", async () => {
    const run = vi.fn<RunInTerminal>().mockRejectedValue(new Error("Workspace terminal is not open"));
    render("```sh\nmake\n```", run);
    await act(async () => { runButton()!.click(); });
    expect(runButton()?.textContent).toBe("Couldn't run");
    expect(runButton()?.dataset.tip).toBe("Workspace terminal is not open");
    expect(runButton()?.getAttribute("aria-label")).toBe("Couldn't run: Workspace terminal is not open");
    act(() => { vi.advanceTimersByTime(3_000); });
    expect(runButton()?.textContent).toBe("Run");
  });

  it("counts the lines a multi-line block will run", () => {
    render("```bash\ncd app\nbun install\nbun run dev\n```", vi.fn<RunInTerminal>());
    expect(runButton()?.dataset.tip).toBe("Run these 3 lines in the terminal");
  });
});
