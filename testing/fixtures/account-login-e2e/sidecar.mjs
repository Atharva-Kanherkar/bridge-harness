// Fake Claude sidecar: answers Bridge's no-turn usage probe from the login the
// fake `claude auth login` left in CLAUDE_CONFIG_DIR.
import fs from "node:fs";
import path from "node:path";
const request = JSON.parse(process.argv[2] ?? "{}");
if (!request.usage) process.exit(1);
const dir = process.env.CLAUDE_CONFIG_DIR;
let login = null;
try { login = JSON.parse(fs.readFileSync(path.join(dir ?? "", ".e2e-login.json"), "utf8")); } catch {}
if (!login) {
  process.stdout.write(JSON.stringify({ type: "claude_usage_error", code: "initialization_failed" }));
} else {
  const used = (login.email.length * 3) % 80;
  process.stdout.write(JSON.stringify({
    type: "claude_usage", rateLimitsAvailable: true,
    account: { email: login.email, subscriptionType: "max" },
    rateLimits: { five_hour: { utilization: used, resets_at: "2100-01-01T00:00:00Z" }, seven_day: { utilization: used / 2, resets_at: "2100-01-01T00:00:00Z" } },
  }));
}
