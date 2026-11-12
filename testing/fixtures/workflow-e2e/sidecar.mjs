// Bridge's real Claude sidecar, run against the fake SDK beside this file.
// Point BRIDGE_CLAUDE_SIDECAR here: everything but the model is real.
import { fileURLToPath } from "node:url";

process.env.BRIDGE_CLAUDE_SDK_ENTRY = fileURLToPath(new URL("./fake-sdk.mjs", import.meta.url));
await import("../../../sidecar/claude-agent/index.mjs");
