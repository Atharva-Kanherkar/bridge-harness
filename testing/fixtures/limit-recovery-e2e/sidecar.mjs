// Bridge's real Claude sidecar, run against the fake SDK beside this file.
// Point BRIDGE_CLAUDE_SIDECAR here: everything but the model is real.
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const settings = mkdtempSync(join(tmpdir(), "limit-e2e-settings-"));
process.env.CLAUDE_CONFIG_DIR = settings;
process.on("exit", () => rmSync(settings, { recursive: true, force: true }));

process.env.BRIDGE_CLAUDE_SDK_ENTRY = fileURLToPath(new URL("./fake-sdk.mjs", import.meta.url));
await import("../../../sidecar/claude-agent/index.mjs");
