import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { errorMessage } from "./errors";
import { ERROR_CODES, PROTOCOL_VERSION } from "./protocol/generated/protocol";
import { CONNECTION_LOST, DAEMON_UNREACHABLE, MAX_FRAME_BYTES, WebDaemonClient, WEB_DAEMON_STORAGE_KEY, activeWebDaemon, followPastedLinks, readWebDaemonConfig, setActiveWebDaemon, type SocketLike, type WebDaemonStatus } from "./webDaemon";

type Frame = { jsonrpc: string; id?: number; method: string; params?: unknown };

class FakeSocket implements SocketLike {
  onopen: ((event: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event: unknown) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  readonly sent: Frame[] = [];
  closed = false;
  constructor(readonly daemon: FakeDaemon, readonly index: number) {}
  send(data: string) {
    const frame = JSON.parse(data) as Frame;
    this.sent.push(frame);
    queueMicrotask(() => this.daemon.handle(this, frame));
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    queueMicrotask(() => this.onclose?.({}));
  }
  reply(id: number, result: unknown) { this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", id, result }) }); }
  fail(id: number, code: number, message: string) { this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }) }); }
  notify(method: string, params?: unknown) { this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", method, ...(params === undefined ? {} : { params }) }) }); }
}

/** A daemon in memory: answers handshakes, hands other requests to `onCall`. */
class FakeDaemon {
  sockets: FakeSocket[] = [];
  up = true;
  /** Connection slots left, like `bridged`'s shared cap; beyond it an upgrade fails. */
  capacity = Number.POSITIVE_INFINITY;
  private opened = new Set<FakeSocket>();
  token = "secret";
  onCall: (socket: FakeSocket, frame: Frame) => void = (socket, frame) => socket.reply(frame.id!, { echo: frame.method });
  factory = (_url: string): SocketLike => {
    const socket = new FakeSocket(this, this.sockets.length);
    this.sockets.push(socket);
    queueMicrotask(() => {
      const full = [...this.opened].filter(open => !open.closed).length >= this.capacity;
      if (this.up && !full) { this.opened.add(socket); socket.onopen?.({}); }
      else { socket.closed = true; socket.onerror?.({}); socket.onclose?.({}); }
    });
    return socket;
  };
  handle(socket: FakeSocket, frame: Frame) {
    if (socket.closed) return;
    if (frame.method === "protocol/handshake") {
      const params = frame.params as { authToken?: string };
      if (params.authToken !== this.token) {
        socket.fail(frame.id!, ERROR_CODES.unauthorized, "the handshake must carry the token from the daemon's daemon.token file");
        socket.close();
        return;
      }
      socket.reply(frame.id!, { protocolVersion: PROTOCOL_VERSION, capabilities: [], server: { name: "bridged", version: "test" } });
      return;
    }
    this.onCall(socket, frame);
  }
  live() { return this.sockets.filter(socket => !socket.closed); }
  dropAll() { for (const socket of this.live()) socket.close(); }
}

const fast = { backoffInitialMs: 1, backoffMaxMs: 5, linkWaitMs: 200, connectTimeoutMs: 200 };

function client(daemon: FakeDaemon, token = "secret", options = {}) {
  return new WebDaemonClient({ url: "ws://127.0.0.1:4319", token }, { socketFactory: daemon.factory, ...fast, ...options });
}

const flush = () => new Promise(resolve => setTimeout(resolve, 5));

async function until(check: () => boolean, ms = 500) {
  const end = Date.now() + ms;
  while (!check()) {
    if (Date.now() > end) throw new Error("condition not reached");
    await flush();
  }
}

describe("WebDaemonClient", () => {
  it("sends the handshake first with the token and protocol version", async () => {
    const daemon = new FakeDaemon();
    const web = client(daemon).start();
    await web.call("health/health");
    expect(daemon.sockets).toHaveLength(7);
    for (const socket of daemon.sockets) {
      expect(socket.sent[0]).toEqual({
        jsonrpc: "2.0",
        id: 1,
        method: "protocol/handshake",
        params: { authToken: "secret", client: { name: "bridge-web", version: "web" }, protocolVersion: PROTOCOL_VERSION },
      });
    }
    web.stop();
  });

  it("matches responses by id with notifications interleaved", async () => {
    const daemon = new FakeDaemon();
    const seen: unknown[] = [];
    daemon.onCall = (socket, frame) => {
      daemon.sockets[0].notify("state-changed");
      socket.notify("agent-event", { ignored: "call sockets do not deliver" });
      socket.reply(frame.id!, { answer: frame.method });
    };
    const web = client(daemon).start();
    web.subscribe("state-changed", payload => seen.push(payload));
    web.subscribe("agent-event", payload => seen.push(payload));
    await expect(web.call("state/get_state")).resolves.toEqual({ answer: "state/get_state" });
    expect(seen).toEqual([null]);
    web.stop();
  });

  it("omits params for parameterless calls and forwards params verbatim", async () => {
    const daemon = new FakeDaemon();
    const web = client(daemon).start();
    await web.call("health/health");
    await web.call("projects/add_project", { path: "/tmp/repo" });
    const calls = daemon.sockets.flatMap(socket => socket.sent.filter(frame => frame.method !== "protocol/handshake"));
    expect(calls[0]).toEqual({ jsonrpc: "2.0", id: 2, method: "health/health" });
    expect("params" in calls[0]).toBe(false);
    expect(calls[1].params).toEqual({ path: "/tmp/repo" });
    web.stop();
  });

  it("rejects an rpc error with the host error envelope", async () => {
    const daemon = new FakeDaemon();
    daemon.onCall = (socket, frame) => socket.fail(frame.id!, ERROR_CODES.git, "not a git repository");
    const web = client(daemon).start();
    const error = await web.call("workspaces/workspace_changes", { workspaceId: "w" }).catch((cause: unknown) => cause);
    expect(typeof error).toBe("string");
    expect(JSON.parse(error as string)).toEqual({ code: 1001, kind: "git", message: "not a git repository" });
    expect(errorMessage(error)).toBe("not a git repository");
    web.stop();
  });

  it("rejects in-flight calls when the socket closes", async () => {
    const daemon = new FakeDaemon();
    daemon.onCall = socket => socket.close();
    const web = client(daemon).start();
    await expect(web.call("state/get_state")).rejects.toBe(CONNECTION_LOST);
    web.stop();
  });

  it("routes github methods to github lanes and prefers an idle lane", async () => {
    const daemon = new FakeDaemon();
    const held: Array<{ socket: FakeSocket; id: number }> = [];
    daemon.onCall = (socket, frame) => held.push({ socket, id: frame.id! });
    const web = client(daemon).start();
    const calls = [
      web.call("github/github_prs", { workspaceId: "w" }),
      web.call("github/github_issues", { workspaceId: "w" }),
      web.call("health/health"),
      web.call("state/get_state"),
    ];
    await until(() => held.length === 4);
    const lanes = held.map(entry => entry.socket.index);
    // 0 is the events socket, 1-4 general, 5-6 github.
    expect(lanes.slice(0, 2).sort()).toEqual([5, 6]);
    expect(lanes.slice(2).every(index => index >= 1 && index <= 4)).toBe(true);
    expect(new Set(lanes.slice(2)).size).toBe(2);
    for (const entry of held) entry.socket.reply(entry.id, null);
    await Promise.all(calls);
    web.stop();
  });

  it("a slow github call does not block a general call", async () => {
    const daemon = new FakeDaemon();
    daemon.onCall = (socket, frame) => {
      if (!frame.method.startsWith("github/")) socket.reply(frame.id!, "ok");
    };
    const web = client(daemon).start();
    void web.call("github/github_prs", { workspaceId: "w" }).catch(() => undefined);
    void web.call("github/github_issues", { workspaceId: "w" }).catch(() => undefined);
    void web.call("github/github_checks", { workspaceId: "w", number: 1 }).catch(() => undefined);
    await expect(web.call("health/health")).resolves.toBe("ok");
    web.stop();
  });

  it("keeps the sockets that authenticate when the daemon is near its connection cap", async () => {
    for (const [capacity, callSockets] of [[1, 0], [3, 2], [6, 5]] as const) {
      const daemon = new FakeDaemon();
      daemon.capacity = capacity;
      const lanes: number[] = [];
      daemon.onCall = (socket, frame) => { lanes.push(socket.index); socket.reply(frame.id!, frame.method); };
      const web = client(daemon).start();
      const seen: unknown[] = [];
      web.subscribe("state-changed", payload => seen.push(payload));
      await expect(web.call("health/health")).resolves.toBe("health/health");
      await expect(web.call("github/github_prs", { workspaceId: "w" })).resolves.toBe("github/github_prs");
      expect(daemon.live()).toHaveLength(1 + callSockets);
      // With no call sockets the events socket serves calls; otherwise calls
      // stay off it, and GitHub shares the general lanes.
      expect(lanes.every(index => callSockets === 0 ? index === 0 : index > 0)).toBe(true);
      daemon.sockets[0].notify("state-changed");
      expect(seen).toEqual([null]);
      web.stop();
    }
  });

  it("does not retry a pool that came up short", async () => {
    const daemon = new FakeDaemon();
    daemon.capacity = 2;
    const web = client(daemon).start();
    await web.call("health/health");
    const attempts = daemon.sockets.length;
    await flush();
    await web.call("state/get_state");
    expect(daemon.sockets.length).toBe(attempts);
    web.stop();
  });

  it("refuses an oversized request locally and keeps the other lanes and events", async () => {
    const daemon = new FakeDaemon();
    const held: Array<{ socket: FakeSocket; id: number }> = [];
    daemon.onCall = (socket, frame) => held.push({ socket, id: frame.id! });
    const web = client(daemon).start();
    const seen: unknown[] = [];
    web.subscribe("state-changed", payload => seen.push(payload));
    const pending = web.call("state/get_state");
    await until(() => held.length === 1);
    // Multi-byte text, so a character count would pass where the byte count fails.
    const image = "é".repeat(MAX_FRAME_BYTES / 2);
    const error = await web.call("sessions/send_turn", { sessionId: "s", text: image }).catch((cause: unknown) => cause);
    expect(error).toMatch(/^request frame exceeds 1048576 bytes \(sessions\/send_turn is \d+\)$/);
    expect(daemon.sockets.flatMap(socket => socket.sent).some(frame => frame.method === "sessions/send_turn")).toBe(false);
    expect(daemon.live()).toHaveLength(7);
    held[0].socket.reply(held[0].id, "still here");
    await expect(pending).resolves.toBe("still here");
    daemon.sockets[0].notify("state-changed");
    expect(seen).toEqual([null]);
    web.stop();
  });

  it("pins the frame limit to bridged's", () => {
    const source = readFileSync(fileURLToPath(new URL("../src-tauri/bridged/src/lib.rs", import.meta.url)), "utf8");
    const declared = source.match(/pub const MAX_FRAME_BYTES: usize = ([\d_ *]+);/);
    expect(declared, "bridged::MAX_FRAME_BYTES moved").not.toBeNull();
    const value = declared![1].split("*").map(part => Number(part.trim().replaceAll("_", ""))).reduce((product, factor) => product * factor, 1);
    expect(MAX_FRAME_BYTES).toBe(value);
  });

  it("rebuilds the pool after a drop and emits reconciliation in order", async () => {
    const daemon = new FakeDaemon();
    const order: string[] = [];
    const statuses: string[] = [];
    const web = client(daemon).start();
    web.onStatus(status => statuses.push(status.state));
    for (const name of ["state-changed", "adapters-changed", "stream-lagged"]) web.subscribe(name, () => order.push(name));
    await web.call("health/health");
    expect(order).toEqual([]);
    daemon.dropAll();
    await until(() => order.length === 3);
    expect(order).toEqual(["state-changed", "adapters-changed", "stream-lagged"]);
    expect(daemon.live()).toHaveLength(7);
    expect(statuses).toContain("reconnecting");
    expect(statuses.at(-1)).toBe("attached");
    await expect(web.call("health/health")).resolves.toEqual({ echo: "health/health" });
    web.stop();
  });

  it("keeps retrying while the daemon is down and attaches when it returns", async () => {
    const daemon = new FakeDaemon();
    daemon.up = false;
    const web = client(daemon, "secret", { linkWaitMs: 2_000 }).start();
    const pending = web.call("health/health");
    await until(() => daemon.sockets.length >= 14);
    daemon.up = true;
    await expect(pending).resolves.toEqual({ echo: "health/health" });
    web.stop();
  });

  it("turns stream-lagged into reconciliation", async () => {
    const daemon = new FakeDaemon();
    const order: string[] = [];
    const web = client(daemon).start();
    for (const name of ["state-changed", "adapters-changed", "stream-lagged"]) web.subscribe(name, () => order.push(name));
    await web.call("health/health");
    daemon.sockets[0].notify("stream-lagged", { dropped: 4 });
    expect(order).toEqual(["state-changed", "adapters-changed", "stream-lagged"]);
    web.stop();
  });

  it("stops reconnecting after unauthorized and surfaces the message", async () => {
    const daemon = new FakeDaemon();
    let status: WebDaemonStatus | undefined;
    const web = client(daemon, "wrong").start();
    web.onStatus(next => { status = next; });
    const error = await web.call("health/health").catch((cause: unknown) => cause);
    expect(errorMessage(error)).toBe("the handshake must carry the token from the daemon's daemon.token file");
    expect(status).toEqual({ state: "refused", message: "the handshake must carry the token from the daemon's daemon.token file" });
    const attempts = daemon.sockets.length;
    await flush();
    await flush();
    expect(daemon.sockets.length).toBe(attempts);
    await expect(web.call("state/get_state")).rejects.toMatch(/unauthorized/);
    web.stop();
  });

  it("attaches after the first link wait expired and reports it", async () => {
    const daemon = new FakeDaemon();
    daemon.up = false;
    const statuses: string[] = [];
    const web = client(daemon, "secret", { linkWaitMs: 30 }).start();
    web.onStatus(status => statuses.push(status.state));
    await expect(web.call("health/health")).rejects.toBe(DAEMON_UNREACHABLE);
    daemon.up = true;
    await until(() => statuses.at(-1) === "attached");
    expect(statuses[0]).toBe("connecting");
    expect(statuses).toContain("reconnecting");
    await expect(web.call("health/health")).resolves.toEqual({ echo: "health/health" });
    web.stop();
  });

  it("waits for a link, then rejects as unreachable", async () => {
    const daemon = new FakeDaemon();
    daemon.up = false;
    const web = client(daemon, "secret", { linkWaitMs: 30 }).start();
    await expect(web.call("health/health")).rejects.toBe(DAEMON_UNREACHABLE);
    web.stop();
  });
});

describe("readWebDaemonConfig", () => {
  function harness(hash: string, stored: string | null = null) {
    const storage = new Map<string, string>(stored ? [[WEB_DAEMON_STORAGE_KEY, stored]] : []);
    const replaced: string[] = [];
    const read = () => readWebDaemonConfig(
      { hash, pathname: "/", search: "?x=1" },
      { getItem: key => storage.get(key) ?? null, setItem: (key, value) => { storage.set(key, value); } },
      { replaceState: (_data: unknown, _unused: string, url?: string | URL | null) => { replaced.push(String(url)); } },
    );
    return { storage, replaced, read };
  }

  it("reads the fragment, stores it, and strips only its own keys", () => {
    const { storage, replaced, read } = harness("#bridge-daemon=ws%3A%2F%2F127.0.0.1%3A4319&bridge-token=t%2Bk&keep=1");
    expect(read()).toEqual({ url: "ws://127.0.0.1:4319", token: "t+k" });
    expect(replaced).toEqual(["/?x=1#keep=1"]);
    expect(JSON.parse(storage.get(WEB_DAEMON_STORAGE_KEY)!)).toEqual({ url: "ws://127.0.0.1:4319", token: "t+k" });
  });

  it("drops the fragment entirely when nothing else was in it", () => {
    const { replaced, read } = harness("#bridge-daemon=wss://bridge.example.com&bridge-token=abc");
    expect(read()).toEqual({ url: "wss://bridge.example.com", token: "abc" });
    expect(replaced).toEqual(["/?x=1"]);
  });

  it("falls back to the stored connection on reload", () => {
    const { read, replaced } = harness("", JSON.stringify({ url: "ws://127.0.0.1:4319", token: "abc" }));
    expect(read()).toEqual({ url: "ws://127.0.0.1:4319", token: "abc" });
    expect(replaced).toEqual([]);
  });

  it("rejects anything but ws and wss, and still strips the token", () => {
    const { read, replaced, storage } = harness("#bridge-daemon=https://evil.example&bridge-token=abc");
    expect(read()).toBeNull();
    expect(replaced).toEqual(["/?x=1"]);
    expect(storage.size).toBe(0);
  });

  it("is mock mode with nothing configured or garbage stored", () => {
    expect(harness("").read()).toBeNull();
    expect(harness("", "{not json").read()).toBeNull();
    expect(harness("#other=1").read()).toBeNull();
  });
});

describe("followPastedLinks", () => {
  function tab(hash: string) {
    const storage = new Map<string, string>();
    const replaced: string[] = [];
    let onHashChange: (() => void) | undefined;
    const location = { hash, pathname: "/", search: "" };
    const target = {
      location,
      sessionStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => { storage.set(key, value); } },
      history: { replaceState: (_data: unknown, _unused: string, url?: string | URL | null) => { replaced.push(String(url)); } },
      addEventListener: (type: string, listener: () => void) => { if (type === "hashchange") onHashChange = listener; },
    };
    const reload = vi.fn();
    followPastedLinks(target as unknown as Parameters<typeof followPastedLinks>[0], reload);
    return { location, storage, replaced, reload, change: (next: string) => { location.hash = next; onHashChange?.(); } };
  }

  it("stores a pasted link, strips it, and reloads onto it", () => {
    const pasted = tab("");
    pasted.change("#bridge-daemon=ws%3A%2F%2F127.0.0.1%3A4319&bridge-token=new");
    expect(JSON.parse(pasted.storage.get(WEB_DAEMON_STORAGE_KEY)!)).toEqual({ url: "ws://127.0.0.1:4319", token: "new" });
    expect(pasted.replaced).toEqual(["/"]);
    expect(pasted.reload).toHaveBeenCalledTimes(1);
  });

  it("ignores hash changes that are not a daemon link", () => {
    const other = tab("");
    other.change("#section-2");
    expect(other.reload).not.toHaveBeenCalled();
    expect(other.storage.size).toBe(0);
  });
});

describe("activeWebDaemon", () => {
  it("is mock mode, not a crash, on a partial window", () => {
    vi.stubGlobal("window", {});
    vi.stubGlobal("WebSocket", class {});
    setActiveWebDaemon(undefined);
    try {
      expect(activeWebDaemon()).toBeNull();
    } finally {
      setActiveWebDaemon(undefined);
      vi.unstubAllGlobals();
    }
  });
});
