/** The agent whose MCP connection a personal app uses, as the person knows it. */
const CONNECTION_LABELS: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", cursor: "Cursor" };
export const connectionLabel = (harness: string) => CONNECTION_LABELS[harness] ?? harness;

export const LIVE_READS_UNAVAILABLE = "Live reads run only in the Bridge Mac app, where a preview cannot reach the network. Writes still show what would be sent.";
