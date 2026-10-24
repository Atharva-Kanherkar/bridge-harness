#!/usr/bin/env node
// Print the link that opens Bridge in a browser tab against a running
// `bridged --listen`. The token rides in the URL fragment, which the browser
// never sends to a server; the app moves it into sessionStorage and strips it
// from the address bar on load.
//
//   bun run web:link -- --data-dir <dir> [--daemon ws://127.0.0.1:4319] [--app http://127.0.0.1:1420]
//
// Start the daemon with an allowlist naming the app's origin:
//
//   bridged --data-dir <dir> --listen 127.0.0.1:4319 --allowed-origin http://127.0.0.1:1420

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_DAEMON = "ws://127.0.0.1:4319";
export const DEFAULT_APP = "http://127.0.0.1:1420";
const TOKEN_FILE = "daemon.token";

export function readToken(dataDir) {
  const path = join(dataDir, TOKEN_FILE);
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch (error) {
    throw new Error(`no ${TOKEN_FILE} in ${dataDir}: start bridged with this --data-dir once to create it (${error.code ?? error.message})`);
  }
  const token = raw.trim();
  if (!/^[0-9a-f]{64}$/i.test(token)) throw new Error(`${path} is not a 64-character hex token`);
  return token;
}

export function webLink({ app = DEFAULT_APP, daemon = DEFAULT_DAEMON, token }) {
  const scheme = new URL(daemon).protocol;
  if (scheme !== "ws:" && scheme !== "wss:") throw new Error(`--daemon must be a ws:// or wss:// URL, got ${daemon}`);
  const base = new URL(app);
  base.hash = "";
  return `${base.toString()}#bridge-daemon=${encodeURIComponent(daemon)}&bridge-token=${encodeURIComponent(token)}`;
}

export function parseArgs(argv, env = process.env) {
  const options = { dataDir: env.BRIDGE_DATA_DIR, daemon: DEFAULT_DAEMON, app: DEFAULT_APP };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (flag === "--") continue;
    if (!["--data-dir", "--daemon", "--app"].includes(flag)) throw new Error(`unknown argument ${flag}`);
    if (value === undefined) throw new Error(`${flag} needs a value`);
    index += 1;
    if (flag === "--data-dir") options.dataDir = value;
    if (flag === "--daemon") options.daemon = value;
    if (flag === "--app") options.app = value;
  }
  if (!options.dataDir) throw new Error("--data-dir is required (or set BRIDGE_DATA_DIR)");
  return options;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const options = parseArgs(process.argv.slice(2));
    const link = webLink({ app: options.app, daemon: options.daemon, token: readToken(options.dataDir) });
    process.stderr.write(`The daemon must allow this origin: --allowed-origin ${new URL(options.app).origin}\n`);
    process.stdout.write(`${link}\n`);
  } catch (error) {
    process.stderr.write(`web-link: ${error.message}\n`);
    process.exit(1);
  }
}
