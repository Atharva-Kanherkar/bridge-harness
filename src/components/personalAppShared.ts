/** The agent whose MCP connection a personal app uses, as the person knows it. */
const CONNECTION_LABELS: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", cursor: "Cursor" };
export const connectionLabel = (harness: string) => CONNECTION_LABELS[harness] ?? harness;

/** What the person should know about a connection that is not a direct call. */
export const connectionNote = (harness: string): string | undefined => harness === "opencode"
  ? "Less safe: this app runs through OpenCode, whose code mode can also make network requests Bridge cannot approve. Bridge still allows only the exact MCP call shown; use it at your own discretion. Each call uses a model turn, so it is slower, and its tools are not verified."
  : undefined;

export const LIVE_READS_UNAVAILABLE = "Live reads run only in the Bridge Mac app, where a preview cannot reach the network. Writes still show what would be sent.";
