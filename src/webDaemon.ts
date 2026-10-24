// The browser's way to a real `bridged`: JSON-RPC over the daemon's opt-in
// WebSocket listener (`bridged --listen`). It is the TypeScript twin of the
// desktop shell's `src-tauri/src/daemon_host.rs`, with the same pool shape,
// lane rules and refetch hints after a reconnect, so `api.ts` can route to it
// without any component knowing which transport it is on.

import { ERROR_CODES, HANDSHAKE_METHOD, PROTOCOL_VERSION } from "./protocol/generated/protocol";

/** Where the daemon is and the token its handshake wants. */
export type WebDaemonConfig = { url: string; token: string };

export type WebDaemonStatus =
  | { state: "connecting" }
  | { state: "attached" }
  | { state: "reconnecting"; reason: string }
  /** The daemon said no (bad token, incompatible protocol). Retrying cannot help. */
  | { state: "refused"; message: string };

/** The slice of the DOM `WebSocket` this module uses, so tests can fake it. */
export interface SocketLike {
  send(data: string): void;
  close(): void;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: unknown) => void) | null;
  onerror: ((event: unknown) => void) | null;
}
export type SocketFactory = (url: string) => SocketLike;

type Timer = ReturnType<typeof setTimeout>;
export type WebDaemonOptions = {
  socketFactory?: SocketFactory;
  clientVersion?: string;
  generalLanes?: number;
  githubLanes?: number;
  /** How long a call made mid-reconnect waits for the link (`CALL_LINK_WAIT`). */
  linkWaitMs?: number;
  /** One call's budget, matching bridge-client's `DEFAULT_CALL_TIMEOUT`. */
  callTimeoutMs?: number;
  connectTimeoutMs?: number;
  backoffInitialMs?: number;
  backoffMaxMs?: number;
};

// Match daemon_host.rs. The daemon answers a connection's requests one at a
// time, so lanes are what keep a slow `gh` read from queueing a session or
// health call behind it.
const GENERAL_LANES = 4;
const GITHUB_LANES = 2;
const CALL_LINK_WAIT_MS = 10_000;
const CALL_TIMEOUT_MS = 300_000;
const CONNECT_TIMEOUT_MS = 30_000;
const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 5_000;

/** `bridged::MAX_FRAME_BYTES`: the daemon closes a connection whose message is
 *  larger, taking every call on it down too. A test pins this to the Rust value. */
export const MAX_FRAME_BYTES = 1024 * 1024;
const utf8 = new TextEncoder();

export const CONNECTION_LOST = "The Bridge daemon connection was lost; reconnecting";
export const DAEMON_UNREACHABLE = "The Bridge daemon is unreachable";

const ERROR_KIND_BY_CODE = new Map<number, string>(
  Object.entries(ERROR_CODES).map(([name, code]) => [code, name]),
);

type RpcError = { code: number; message: string };

/** The same string `daemon_host::host_error_envelope` rejects with, so
 *  `errorMessage()` renders a web error exactly like a desktop one. */
export function hostErrorEnvelope(error: RpcError): string {
  return JSON.stringify({ code: error.code, kind: ERROR_KIND_BY_CODE.get(error.code) ?? null, message: error.message });
}

/** A handshake the daemon refused on purpose. Reconnecting will not fix it. */
class HandshakeRefused extends Error {
  constructor(readonly envelope: string) { super(envelope); }
}

const REFUSAL_CODES = new Set<number>([ERROR_CODES.unauthorized, ERROR_CODES.incompatible_protocol]);

// Every rejection a caller sees is a string, as a Tauri `invoke` rejection is:
// `errorMessage()` only unwraps the `{code,kind,message}` envelope from one.
type Pending = { resolve: (value: unknown) => void; reject: (reason: unknown) => void; timer: Timer };

/** One WebSocket: handshake first, then calls matched by id. */
export class DaemonSocket {
  private socket: SocketLike | null = null;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private closed = false;
  private closeHandlers = new Set<() => void>();

  constructor(
    private readonly factory: SocketFactory,
    private readonly config: WebDaemonConfig,
    private readonly onNotification: (method: string, params: unknown) => void,
    private readonly options: { clientVersion: string; callTimeoutMs: number; connectTimeoutMs: number },
  ) {}

  /** Open and handshake. Resolves once the daemon accepted the token. */
  open(): Promise<void> {
    return new Promise((resolve, reject) => {
      let settled = false;
      const fail = (reason: unknown) => {
        if (settled) return;
        settled = true;
        clearTimeout(deadline);
        this.close();
        reject(reason);
      };
      const deadline = setTimeout(() => fail(new Error(`${this.config.url} did not accept a connection in time`)), this.options.connectTimeoutMs);
      let socket: SocketLike;
      try {
        socket = this.factory(this.config.url);
      } catch (error) {
        fail(error);
        return;
      }
      this.socket = socket;
      socket.onmessage = event => this.receive(event.data);
      socket.onerror = () => undefined;
      socket.onclose = () => {
        fail(new Error(`${this.config.url} closed before the handshake`));
        this.shutdown();
      };
      socket.onopen = () => {
        this.request(HANDSHAKE_METHOD, {
          authToken: this.config.token,
          client: { name: "bridge-web", version: this.options.clientVersion },
          protocolVersion: PROTOCOL_VERSION,
        }).then(
          () => {
            if (settled) return;
            settled = true;
            clearTimeout(deadline);
            resolve();
          },
          (error: unknown) => {
            const envelope = String(error);
            fail(isRefusal(envelope) ? new HandshakeRefused(envelope) : new Error(messageOf(envelope)));
          },
        );
      };
    });
  }

  /** In flight on this socket. The daemon serves them in order, so busy = queued. */
  get busy(): boolean {
    return this.pending.size > 0;
  }

  request(method: string, params?: unknown): Promise<unknown> {
    if (this.closed || !this.socket) return Promise.reject(CONNECTION_LOST);
    const id = this.nextId++;
    // A parameterless method sends no `params`: the daemon rejects a stray
    // payload, exactly as `daemon_host::wire_params` guards on the desktop.
    const frame = params === undefined
      ? { jsonrpc: "2.0", id, method }
      : { jsonrpc: "2.0", id, method, params };
    const serialized = JSON.stringify(frame);
    // Refuse it here, as bridge-client does: sent, the daemon would close this
    // socket, and the pool would drop every other call with it.
    const bytes = utf8.encode(serialized).length;
    if (bytes > MAX_FRAME_BYTES) return Promise.reject(`request frame exceeds ${MAX_FRAME_BYTES} bytes (${method} is ${bytes})`);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending.delete(id)) reject(`${method} did not answer in time`);
      }, this.options.callTimeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.socket!.send(serialized);
      } catch {
        this.pending.delete(id);
        clearTimeout(timer);
        reject(CONNECTION_LOST);
      }
    });
  }

  onClose(handler: () => void): void {
    if (this.closed) handler();
    else this.closeHandlers.add(handler);
  }

  close(): void {
    try { this.socket?.close(); } catch { /* already gone */ }
    this.shutdown();
  }

  private receive(data: unknown): void {
    if (typeof data !== "string") return;
    let frame: Record<string, unknown>;
    try { frame = JSON.parse(data); } catch { return; }
    if (!frame || typeof frame !== "object") return;
    if ("id" in frame && ("result" in frame || "error" in frame)) {
      // An id-less error is the daemon refusing the whole connection
      // (overloaded, a frame it could not parse); the close that follows
      // rejects whatever was waiting.
      if (typeof frame.id !== "number") return;
      const pending = this.pending.get(frame.id);
      if (!pending) return;
      this.pending.delete(frame.id);
      clearTimeout(pending.timer);
      if ("error" in frame) pending.reject(hostErrorEnvelope(frame.error as RpcError));
      else pending.resolve(frame.result);
      return;
    }
    if (typeof frame.method === "string" && !("id" in frame)) this.onNotification(frame.method, frame.params ?? null);
  }

  private shutdown(): void {
    if (this.closed) return;
    this.closed = true;
    for (const [id, pending] of this.pending) {
      this.pending.delete(id);
      clearTimeout(pending.timer);
      pending.reject(CONNECTION_LOST);
    }
    const handlers = [...this.closeHandlers];
    this.closeHandlers.clear();
    for (const handler of handlers) handler();
  }
}

function isRefusal(envelope: string): boolean {
  try {
    const parsed = JSON.parse(envelope) as { code?: unknown };
    return typeof parsed.code === "number" && REFUSAL_CODES.has(parsed.code);
  } catch { return false; }
}

type Link = { events: DaemonSocket; general: DaemonSocket[]; github: DaemonSocket[] };
type Waiter = { resolve: (link: Link) => void; reject: (reason: unknown) => void; timer: Timer };

/**
 * A pool of daemon sockets that survives the daemon going away.
 *
 * One socket carries notifications and the rest serve calls, split into
 * general and GitHub lanes. Losing any socket drops the whole pool, which is
 * rebuilt with backoff. After a re-attach, and whenever the daemon reports
 * `stream-lagged`, subscribers get the same refetch hints the desktop host
 * emits, so the UI re-reads state instead of silently missing events.
 */
export class WebDaemonClient {
  private link: Link | null = null;
  private waiters = new Set<Waiter>();
  private readonly listeners = new Map<string, Set<(payload: unknown) => void>>();
  private readonly statusListeners = new Set<(status: WebDaemonStatus) => void>();
  private status: WebDaemonStatus = { state: "connecting" };
  private refusal: string | null = null;
  private stopped = false;
  private started = false;
  private rotation = 0;
  private readonly options: Required<WebDaemonOptions>;

  constructor(private readonly config: WebDaemonConfig, options: WebDaemonOptions = {}) {
    this.options = {
      socketFactory: options.socketFactory ?? (url => new WebSocket(url) as unknown as SocketLike),
      clientVersion: options.clientVersion ?? "web",
      generalLanes: options.generalLanes ?? GENERAL_LANES,
      githubLanes: options.githubLanes ?? GITHUB_LANES,
      linkWaitMs: options.linkWaitMs ?? CALL_LINK_WAIT_MS,
      callTimeoutMs: options.callTimeoutMs ?? CALL_TIMEOUT_MS,
      connectTimeoutMs: options.connectTimeoutMs ?? CONNECT_TIMEOUT_MS,
      backoffInitialMs: options.backoffInitialMs ?? BACKOFF_INITIAL_MS,
      backoffMaxMs: options.backoffMaxMs ?? BACKOFF_MAX_MS,
    };
  }

  get url(): string {
    return this.config.url;
  }

  start(): this {
    if (!this.started) {
      this.started = true;
      void this.run();
    }
    return this;
  }

  stop(): void {
    this.stopped = true;
    this.teardown();
    this.rejectWaiters(DAEMON_UNREACHABLE);
  }

  async call(method: string, params?: unknown): Promise<unknown> {
    const link = await this.waitForLink();
    return this.pickLane(method, link).request(method, params);
  }

  subscribe(name: string, handler: (payload: unknown) => void): () => void {
    let set = this.listeners.get(name);
    if (!set) this.listeners.set(name, set = new Set());
    set.add(handler);
    return () => { set!.delete(handler); };
  }

  onStatus(handler: (status: WebDaemonStatus) => void): () => void {
    this.statusListeners.add(handler);
    handler(this.status);
    return () => { this.statusListeners.delete(handler); };
  }

  private async run(): Promise<void> {
    let backoff = this.options.backoffInitialMs;
    let attachedBefore = false;
    while (!this.stopped) {
      let link: Link;
      try {
        link = await this.connect();
      } catch (error) {
        if (error instanceof HandshakeRefused) {
          this.refusal = error.envelope;
          this.setStatus({ state: "refused", message: messageOf(error.envelope) });
          this.rejectWaiters(error.envelope);
          return;
        }
        if (this.stopped) return;
        this.setStatus({ state: "reconnecting", reason: error instanceof Error ? error.message : String(error) });
        await sleep(backoff);
        backoff = Math.min(backoff * 2, this.options.backoffMaxMs);
        continue;
      }
      if (this.stopped) { closeAll(link); return; }
      backoff = this.options.backoffInitialMs;
      const lost = new Promise<void>(resolve => {
        for (const socket of allSockets(link)) socket.onClose(resolve);
      });
      this.link = link;
      this.setStatus({ state: "attached" });
      for (const waiter of [...this.waiters]) { this.waiters.delete(waiter); clearTimeout(waiter.timer); waiter.resolve(link); }
      if (attachedBefore) this.reconcile();
      attachedBefore = true;
      await lost;
      this.teardown();
      if (!this.stopped) this.setStatus({ state: "reconnecting", reason: CONNECTION_LOST });
    }
  }

  private async connect(): Promise<Link> {
    const socket = (delivers: boolean) => new DaemonSocket(
      this.options.socketFactory,
      this.config,
      // Every socket hears every notification; only the events socket's copy
      // is delivered, as on the desktop.
      delivers ? (method, params) => this.deliver(method, params) : () => undefined,
      this.options,
    );
    // The first socket is the requirement: it proves the token and carries the
    // events. A refusal or an unreachable daemon ends the attempt here.
    const events = socket(true);
    await events.open();
    // The rest are an optimization, as in `daemon_host::attach`: the daemon caps
    // Unix and WebSocket clients together, so near that cap some of these fail,
    // and the pool is laid out from whichever authenticated.
    const extras = Array.from({ length: this.options.generalLanes + this.options.githubLanes }, () => socket(false));
    const opened = await Promise.allSettled(extras.map(extra => extra.open()));
    const calls = extras.filter((_, index) => opened[index].status === "fulfilled");
    return layOutLanes(events, calls, this.options.generalLanes);
  }

  private deliver(method: string, params: unknown): void {
    if (method === "stream-lagged") {
      this.reconcile();
      return;
    }
    this.emit(method, params);
  }

  /** `daemon_host::emit_reconciliation`, verbatim. */
  private reconcile(): void {
    this.emit("state-changed", null);
    this.emit("adapters-changed", null);
    this.emit("stream-lagged", null);
  }

  private emit(name: string, payload: unknown): void {
    for (const handler of [...(this.listeners.get(name) ?? [])]) {
      try { handler(payload); } catch (error) { console.error(`bridge-web: ${name} handler failed`, error); }
    }
  }

  /** GitHub methods stay on their lanes and everything else stays off them;
   *  an idle lane wins, and only when all are busy does rotation decide. */
  private pickLane(method: string, link: Link): DaemonSocket {
    const lanes = method.startsWith("github/") ? link.github : link.general;
    const start = this.rotation++ % lanes.length;
    for (let offset = 0; offset < lanes.length; offset += 1) {
      const lane = lanes[(start + offset) % lanes.length];
      if (!lane.busy) return lane;
    }
    return lanes[start];
  }

  private waitForLink(): Promise<Link> {
    if (this.refusal) return Promise.reject(this.refusal);
    if (this.link) return Promise.resolve(this.link);
    if (this.stopped) return Promise.reject(DAEMON_UNREACHABLE);
    this.start();
    return new Promise((resolve, reject) => {
      const waiter: Waiter = {
        resolve,
        reject,
        timer: setTimeout(() => {
          this.waiters.delete(waiter);
          reject(DAEMON_UNREACHABLE);
        }, this.options.linkWaitMs),
      };
      this.waiters.add(waiter);
    });
  }

  private rejectWaiters(reason: string): void {
    for (const waiter of [...this.waiters]) { this.waiters.delete(waiter); clearTimeout(waiter.timer); waiter.reject(reason); }
  }

  private teardown(): void {
    const link = this.link;
    this.link = null;
    if (link) closeAll(link);
  }

  private setStatus(status: WebDaemonStatus): void {
    this.status = status;
    for (const handler of [...this.statusListeners]) handler(status);
  }
}

/**
 * `split_event_connection` and `lane_range`, for a pool that may have come up
 * short. With no call sockets the events socket serves calls too. With no more
 * call sockets than general lanes, GitHub calls share them; only a pool with
 * room to spare reserves GitHub lanes.
 */
function layOutLanes(events: DaemonSocket, calls: DaemonSocket[], generalLanes: number): Link {
  if (calls.length === 0) return { events, general: [events], github: [events] };
  if (calls.length <= generalLanes) return { events, general: calls, github: calls };
  return { events, general: calls.slice(0, generalLanes), github: calls.slice(generalLanes) };
}

function allSockets(link: Link): DaemonSocket[] {
  return [...new Set([link.events, ...link.general, ...link.github])];
}

function closeAll(link: Link): void {
  for (const socket of allSockets(link)) socket.close();
}

function messageOf(envelope: string): string {
  try {
    const parsed = JSON.parse(envelope) as { message?: unknown };
    if (typeof parsed.message === "string") return parsed.message;
  } catch { /* not an envelope */ }
  return envelope;
}

const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

// --- connecting a tab ------------------------------------------------------

export const WEB_DAEMON_STORAGE_KEY = "bridge.webDaemon";
const DAEMON_PARAM = "bridge-daemon";
const TOKEN_PARAM = "bridge-token";

type LocationLike = Pick<Location, "hash" | "pathname" | "search">;
type HistoryLike = Pick<History, "replaceState">;
type StorageLike = Pick<Storage, "getItem" | "setItem">;

function validConfig(value: unknown): WebDaemonConfig | null {
  if (!value || typeof value !== "object") return null;
  const { url, token } = value as { url?: unknown; token?: unknown };
  if (typeof url !== "string" || typeof token !== "string") return null;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") return null;
  } catch { return null; }
  return { url, token };
}

/**
 * The daemon this tab should talk to, or null for mock mode.
 *
 * A link carries it in the fragment (`#bridge-daemon=…&bridge-token=…`),
 * which browsers never send to a server. It is read once, kept in
 * `sessionStorage` so a reload stays connected and closing the tab forgets
 * it, and removed from the address bar so the token is not left in history.
 */
export function readWebDaemonConfig(location: LocationLike, storage: StorageLike, history: HistoryLike): WebDaemonConfig | null {
  const fragment = new URLSearchParams(location.hash.replace(/^#/, ""));
  if (fragment.has(DAEMON_PARAM)) {
    const config = validConfig({ url: fragment.get(DAEMON_PARAM) ?? "", token: fragment.get(TOKEN_PARAM) ?? "" });
    fragment.delete(DAEMON_PARAM);
    fragment.delete(TOKEN_PARAM);
    const rest = fragment.toString();
    history.replaceState(null, "", `${location.pathname}${location.search}${rest ? `#${rest}` : ""}`);
    if (config) {
      try { storage.setItem(WEB_DAEMON_STORAGE_KEY, JSON.stringify(config)); } catch { /* storage off: this load still connects */ }
    }
    return config;
  }
  try {
    const stored = storage.getItem(WEB_DAEMON_STORAGE_KEY);
    return stored ? validConfig(JSON.parse(stored)) : null;
  } catch { return null; }
}

/**
 * A link pasted into a tab that is already open only changes the hash, so the
 * page does not load again and the fragment, token included, would sit in the
 * address bar while the old connection carried on. Take it the same way a
 * fresh load would: store it, strip it, and reload onto it.
 */
export function followPastedLinks(target: Pick<Window, "addEventListener" | "location" | "sessionStorage" | "history">, reload: () => void): void {
  target.addEventListener("hashchange", () => {
    if (!new URLSearchParams(target.location.hash.replace(/^#/, "")).has(DAEMON_PARAM)) return;
    try { readWebDaemonConfig(target.location, target.sessionStorage, target.history); } catch { /* storage off */ }
    reload();
  });
}

let active: WebDaemonClient | null | undefined;

/** The tab's daemon client, created on first use; null outside a browser,
 *  inside the desktop shell, or with no daemon configured. */
export function activeWebDaemon(): WebDaemonClient | null {
  if (active !== undefined) return active;
  if (typeof window === "undefined" || "__TAURI_INTERNALS__" in window || typeof WebSocket === "undefined") return active = null;
  let config: WebDaemonConfig | null = null;
  // Reading `sessionStorage` can throw (storage disabled, sandboxed frame), and
  // a partial window has no events to follow; either is mock mode, not a crash.
  try {
    followPastedLinks(window, () => window.location.reload());
    config = readWebDaemonConfig(window.location, window.sessionStorage, window.history);
  } catch { config = null; }
  return active = config ? new WebDaemonClient(config).start() : null;
}

/** Tests only: pin the active client (or mock mode with null; reset with undefined). */
export function setActiveWebDaemon(client: WebDaemonClient | null | undefined): void {
  active = client;
}
