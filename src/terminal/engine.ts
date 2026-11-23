import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openExternalUrl } from "../externalLinks";
import { bridgeApi } from "../api";
import { terminalTheme } from "./theme";
import { attachWebgl } from "./webgl";
import { TerminalReplay } from "./replay";
import { TerminalInput } from "./input";
import { TERMINAL_SCROLLBACK, type TerminalRecord } from "./types";

export const TERMINAL_FONT = "Geist Mono Variable";
export const TERMINAL_FONT_SIZE = 12;
const FONT_FAMILY = `'${TERMINAL_FONT}', ui-monospace, SFMono-Regular, Menlo, monospace`;
/** The same family list spelled differently: assigning it is how xterm is
 *  told to measure its cell again once a late font arrives. */
const FONT_FAMILY_REMEASURE = FONT_FAMILY.replaceAll("'", '"');
const FONT_SPEC = `${TERMINAL_FONT_SIZE}px "${TERMINAL_FONT}"`;
/** How long a first open waits for the web font before measuring anyway. */
const FONT_WAIT_MS = 1500;

/** Resolves once the terminal font can be measured, or after a bound.
 *  xterm measures its cell once, at open: measured on the fallback font,
 *  every glyph then overlaps or gaps until the next resize. */
export function terminalFontReady(): Promise<void> {
  const fonts = typeof document === "undefined" ? undefined : document.fonts;
  if (!fonts?.load) return Promise.resolve();
  return Promise.race([
    fonts.load(FONT_SPEC).then(() => undefined, () => undefined),
    new Promise<void>(resolve => { setTimeout(resolve, FONT_WAIT_MS); }),
  ]);
}

export type TerminalViewState = { restoring: boolean; error?: string };

/** The React view currently showing an engine. */
export type TerminalView = {
  record: (record: TerminalRecord) => void;
  state: (state: TerminalViewState) => void;
  matches: (summary: string) => void;
};

type Identity = Pick<TerminalRecord, "workspaceId" | "terminalId" | "generation" | "status">;

const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));

/** One live xterm bound to one terminal generation: the runtime checkpoint
 *  plus sequenced frames in, ordered input out, sized to whatever host it is
 *  attached to. It outlives a React remount (see `releaseTerminalEngine`), so
 *  splitting a pane or switching a shell tab never rebuilds the screen. */
export class TerminalEngine {
  readonly terminal: Terminal;
  readonly search = new SearchAddon();
  /** xterm's parent, and the element FitAddon measures. It never carries
   *  padding: under border-box a padded parent reports its padding as room,
   *  and the bottom row and right column of every TUI were drawn clipped. */
  readonly element: HTMLDivElement;
  readonly key: string;
  private readonly workspaceId: string;
  private readonly terminalId: string;
  private readonly fit = new FitAddon();
  private readonly input: TerminalInput;
  private readonly feed: TerminalReplay;
  private readonly cleanups: (() => void)[] = [];
  private unlisten: (() => void)[] = [];
  private view?: TerminalView;
  private viewState: TerminalViewState = { restoring: true };
  private status: string;
  private replaying = true;
  private opened = false;
  private opening = false;
  private focused = false;
  private disposed = false;
  private lastSize = "";
  private resizing = false;
  private pendingSize?: { rows: number; cols: number };
  private frame = 0;
  private gpu: () => void = () => {};

  constructor(identity: Identity) {
    const { workspaceId, terminalId } = identity;
    this.workspaceId = workspaceId;
    this.terminalId = terminalId;
    this.key = engineKey(identity);
    this.status = identity.status;
    this.element = document.createElement("div");
    this.element.className = "h-full w-full";
    this.terminal = new Terminal({
      fontFamily: FONT_FAMILY,
      fontSize: TERMINAL_FONT_SIZE,
      lineHeight: 1.35,
      cursorBlink: true,
      cursorStyle: "bar",
      scrollback: TERMINAL_SCROLLBACK,
      allowProposedApi: true,
      theme: terminalTheme(),
      vtExtensions: { kittyKeyboard: true },
      // Option-drag selects text even while a TUI has the mouse; without it a
      // macOS user could never copy out of Claude Code, vim, or htop.
      macOptionClickForcesSelection: true,
    });
    this.terminal.loadAddon(this.fit);
    this.terminal.loadAddon(this.search);
    this.terminal.loadAddon(new Unicode11Addon());
    this.terminal.unicode.activeVersion = "11";
    this.terminal.loadAddon(new WebLinksAddon((event, url) => {
      if ((event.metaKey || event.ctrlKey) && /^https?:\/\//i.test(url)) void openExternalUrl(url).catch(error => this.report(error));
    }));
    this.input = new TerminalInput((data, binary) => bridgeApi.writeTerminal(workspaceId, terminalId, data, binary), error => this.report(error));
    const accepts = () => !this.replaying && this.status === "running";
    const listeners = [
      this.terminal.onData(data => { if (accepts()) this.input.send(data); }),
      // X10 mouse reports (what macOS terminfo selects for ncurses apps) come
      // only through here. Dropping them made clicks and the wheel dead.
      this.terminal.onBinary(data => { if (accepts()) this.input.send(data, true); }),
      this.search.onDidChangeResults(result => this.view?.matches(result.resultCount ? `${result.resultIndex + 1} / ${result.resultCount}` : "No matches")),
    ];
    this.cleanups.push(() => listeners.forEach(listener => listener.dispose()));
    this.terminal.attachCustomKeyEventHandler(event => {
      if (event.type !== "keydown") return true;
      if (event.metaKey && event.key.toLowerCase() === "c" && this.terminal.hasSelection()) {
        void navigator.clipboard.writeText(this.terminal.getSelection()).catch(error => this.report(error));
        return false;
      }
      return true;
    });
    const theme = new MutationObserver(() => { this.terminal.options.theme = terminalTheme(); this.layout(); });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "data-theme"] });
    this.cleanups.push(() => theme.disconnect());
    this.feed = new TerminalReplay({
      snapshot: () => bridgeApi.terminalSnapshot(workspaceId, terminalId),
      restore: snapshot => this.restore(snapshot),
      frame: async frame => {
        if (frame.rows && frame.cols) {
          this.terminal.resize(frame.cols, frame.rows);
          // Another view (the dock, a Fleet pane) sized the PTY to itself.
          // Forget the size this view last sent, so its next fit sends its
          // own again instead of drawing locally at a size the PTY lacks.
          if (`${frame.cols}:${frame.rows}` !== this.lastSize) this.lastSize = "";
        }
        if (frame.data) await this.write(frame.data);
        if (frame.status) {
          this.status = frame.status;
          void bridgeApi.terminalSnapshot(workspaceId, terminalId)
            .then(snapshot => { if (!this.disposed) this.view?.record(snapshot.record); })
            .catch(error => this.report(error));
        }
      },
      error: value => { this.replaying = true; this.setState({ restoring: false }); this.report(value); },
    });
    void this.connect();
  }

  /** Show this engine through `view`; it is told the current state at once. */
  bind(view: TerminalView) {
    this.view = view;
    view.state(this.viewState);
  }

  unbind() { this.view = undefined; }

  attach(host: HTMLElement) {
    host.append(this.element);
    this.layout();
  }

  detach() { this.element.remove(); }

  setFocused(focused: boolean) {
    this.focused = focused;
    if (focused && this.opened) this.terminal.focus();
  }

  retry() { void this.connect(); }

  /** Refit on the next frame. Safe to call on every observer tick. */
  layout() {
    if (this.disposed) return;
    cancelAnimationFrame(this.frame);
    this.frame = requestAnimationFrame(() => this.fitNow());
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.view = undefined;
    cancelAnimationFrame(this.frame);
    this.feed.dispose();
    this.input.close();
    this.unlisten.forEach(fn => fn());
    this.unlisten = [];
    this.cleanups.forEach(fn => fn());
    this.terminal.blur();
    this.gpu();
    this.terminal.dispose();
    this.element.remove();
  }

  private setState(state: TerminalViewState) {
    this.viewState = state;
    this.view?.state(state);
  }

  private report(error: unknown) {
    if (!this.disposed) this.setState({ ...this.viewState, error: String(error) });
  }

  private write(data: string) {
    return new Promise<void>(resolve => this.terminal.write(data, resolve));
  }

  private async restore(snapshot: Awaited<ReturnType<typeof bridgeApi.terminalSnapshot>>) {
    this.replaying = true;
    this.setState({ ...this.viewState, restoring: true });
    this.terminal.reset();
    this.terminal.resize(clamp(snapshot.record.cols, 2, 1000), clamp(snapshot.record.rows, 2, 500));
    await this.write(snapshot.ansi);
    if (this.disposed) return;
    this.status = snapshot.record.status;
    this.view?.record(snapshot.record);
    this.setState({ restoring: false });
    this.replaying = false;
    this.lastSize = "";
    this.layout();
    if (this.focused && this.opened) this.terminal.focus();
  }

  // Install both listeners before requesting the snapshot boundary.
  private async connect() {
    if (this.unlisten.length) { await this.feed.recover(); return; }
    const { workspaceId, terminalId } = this;
    const results = await Promise.allSettled([
      bridgeApi.onTerminalFrame(frame => { if (frame.workspaceId === workspaceId && frame.terminalId === terminalId) this.feed.receive(frame); }),
      bridgeApi.onTerminalLagged(() => { void this.feed.recover(); }),
    ]);
    const listeners = results.flatMap(result => result.status === "fulfilled" ? [result.value] : []);
    const failure = results.find(result => result.status === "rejected");
    if (this.disposed || failure) listeners.forEach(fn => fn());
    if (this.disposed) return;
    if (failure?.status === "rejected") { this.setState({ restoring: false }); this.report(failure.reason); return; }
    this.unlisten.push(...listeners);
    await this.feed.recover();
  }

  private async open() {
    if (this.opening || this.opened) return;
    this.opening = true;
    await terminalFontReady();
    this.opening = false;
    // Hidden again while the font loaded: the next layout tries again.
    if (this.disposed || !this.element.isConnected || !this.element.clientWidth || !this.element.clientHeight) return;
    this.terminal.open(this.element);
    this.opened = true;
    this.gpu = attachWebgl(this.terminal, () => this.layout());
    // The bound above can expire before a slow font arrives. Measured on the
    // fallback font, the grid has too many rows for the pane and the bottom
    // ones are drawn out of sight, so measure again when it lands.
    const fonts = document.fonts;
    if (fonts?.check && fonts.load && !fonts.check(FONT_SPEC)) {
      void fonts.load(FONT_SPEC).then(() => {
        if (this.disposed) return;
        this.terminal.options.fontFamily = FONT_FAMILY_REMEASURE;
        this.layout();
      }, () => undefined);
    }
    if (this.focused) this.terminal.focus();
    this.fitNow();
  }

  private fitNow() {
    if (this.disposed || !this.element.isConnected) return;
    // A hidden host (a closed dock, a background shell tab) measures 0x0.
    // FitAddon would answer 2x1, and xterm would reflow its whole history
    // into two columns, so a hidden terminal is never resized.
    if (!this.element.clientWidth || !this.element.clientHeight) return;
    if (!this.opened) { void this.open(); return; }
    if (this.replaying) return;
    let size = this.fit.proposeDimensions();
    if (!size) {
      // A cell measured while hidden is 0x0 and FitAddon refuses to propose.
      // Resizing to the current size makes xterm measure again.
      this.terminal.resize(this.terminal.cols, this.terminal.rows);
      size = this.fit.proposeDimensions();
    }
    if (!size || !Number.isFinite(size.cols) || !Number.isFinite(size.rows)) return;
    const cols = clamp(size.cols, 2, 1000), rows = clamp(size.rows, 2, 500);
    if (cols !== this.terminal.cols || rows !== this.terminal.rows) this.terminal.resize(cols, rows);
    const key = `${cols}:${rows}`;
    if (this.status === "running" && key !== this.lastSize) {
      this.lastSize = key;
      // Keep only the latest pending dimensions during rapid dragging.
      this.pendingSize = { rows, cols };
      void this.flushResize();
    }
  }

  private async flushResize() {
    if (this.resizing) return;
    this.resizing = true;
    const { workspaceId, terminalId } = this;
    try {
      while (this.pendingSize && !this.disposed) {
        const size = this.pendingSize;
        this.pendingSize = undefined;
        await bridgeApi.resizeTerminal(workspaceId, terminalId, size.rows, size.cols);
      }
    } catch (error) { this.lastSize = ""; this.report(error); }
    finally { this.resizing = false; }
  }
}

export function engineKey(identity: Pick<TerminalRecord, "workspaceId" | "terminalId" | "generation">) {
  return [identity.workspaceId, identity.terminalId, identity.generation].join("\u0000");
}

/** Engines whose view unmounted in this task. A split, a close, or a move
 *  changes the shape of the pane tree and React remounts every pane under it;
 *  the next mount of the same terminal generation adopts the parked engine
 *  instead of disposing xterm and replaying the checkpoint. */
const parked = new Map<string, TerminalEngine>();

export function acquireTerminalEngine(identity: Identity): TerminalEngine {
  const key = engineKey(identity);
  const engine = parked.get(key);
  if (engine) {
    parked.delete(key);
    return engine;
  }
  return new TerminalEngine(identity);
}

/** Park an engine its view no longer shows; it is disposed unless a view of
 *  the same terminal generation adopts it before the next task. Nothing is
 *  kept beyond that, so a hidden tab still holds no GPU context. */
export function releaseTerminalEngine(engine: TerminalEngine) {
  engine.unbind();
  engine.detach();
  const previous = parked.get(engine.key);
  if (previous && previous !== engine) previous.dispose();
  parked.set(engine.key, engine);
  setTimeout(() => {
    if (parked.get(engine.key) !== engine) return;
    parked.delete(engine.key);
    engine.dispose();
  }, 0);
}
