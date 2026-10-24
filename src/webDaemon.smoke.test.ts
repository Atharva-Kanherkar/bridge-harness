import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { errorMessage } from "./errors";
import { WebDaemonClient, type SocketLike } from "./webDaemon";

// Against a real `bridged --listen`. Skipped unless pointed at one:
//
//   bridged --data-dir $D --health-addr none --listen 127.0.0.1:4319 --allowed-origin http://127.0.0.1:1420
//   BRIDGE_WEB_SMOKE_URL=ws://127.0.0.1:4319 BRIDGE_WEB_SMOKE_TOKEN=$(cat $D/daemon.token) \
//     bunx vitest run src/webDaemon.smoke.test.ts
//
// Use a scratch data directory: the last case adds a project to it.
const url = process.env.BRIDGE_WEB_SMOKE_URL;
const token = process.env.BRIDGE_WEB_SMOKE_TOKEN;

const nodeSocket = (address: string) => new WebSocket(address) as unknown as SocketLike;

describe.skipIf(!url || !token)("web daemon smoke", () => {
  const connect = (secret = token!) => new WebDaemonClient({ url: url!, token: secret }, { socketFactory: nodeSocket, linkWaitMs: 5_000 }).start();

  it("handshakes and answers health and state", async () => {
    const web = connect();
    try {
      const health = await web.call("health/health") as { ok: boolean; adapters: unknown[] };
      expect(health.ok).toBe(true);
      expect(Array.isArray(health.adapters)).toBe(true);
      const state = await web.call("state/get_state") as { workspaces: unknown[] };
      expect(Array.isArray(state.workspaces)).toBe(true);
    } finally {
      web.stop();
    }
  });

  it("refuses a wrong token with unauthorized", async () => {
    const web = connect("0".repeat(64));
    try {
      const error = await web.call("health/health").catch((cause: unknown) => cause);
      expect(String(error)).toMatch(/"kind":"unauthorized"/);
      expect(errorMessage(error)).toMatch(/daemon\.token/);
    } finally {
      web.stop();
    }
  });

  it("rejects a bad param with the daemon's own error", async () => {
    const web = connect();
    try {
      const error = await web.call("projects/add_project", { path: 42 }).catch((cause: unknown) => cause);
      expect(String(error)).toMatch(/"kind":"invalid_params"/);
    } finally {
      web.stop();
    }
  });

  it("adds a project that a second connection sees", async () => {
    const repo = mkdtempSync(join(tmpdir(), "bridge-web-smoke-"));
    execFileSync("git", ["init", "-q", repo]);
    const web = connect();
    const other = connect();
    try {
      const name = repo.split("/").at(-1)!;
      const added = await web.call("projects/add_project", { path: repo });
      expect(JSON.stringify(added)).toContain(name);
      expect(JSON.stringify(await other.call("state/get_state"))).toContain(name);
    } finally {
      web.stop();
      other.stop();
      rmSync(repo, { recursive: true, force: true });
    }
  });

  it("delivers state-changed from the events socket after a mutation", async () => {
    const web = connect();
    try {
      await web.call("health/health");
      const changed = new Promise<void>(resolve => { const off = web.subscribe("state-changed", () => { off(); resolve(); }); });
      const created = await web.call("sessions/create_chat_id", { harness: "codex", title: "web smoke" }) as { sessionId: string };
      expect(created.sessionId).toMatch(/[0-9a-f-]{36}/);
      await changed;
    } finally {
      web.stop();
    }
  });
});
