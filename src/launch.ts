// Launch requests from the command line: `bridge-deck --open <id>`,
// `--new-chat`, `--approvals`, `--mission-control`, `--quick`. Launchers,
// notifications and the Omarchy bar use these. The shell parses argv
// (src-tauri/src/launch.rs) and sends one of these as the `bridge-launch` event.

export type LaunchRequest =
  | { kind: "open"; sessionId: string }
  | { kind: "new-chat"; project: string | null }
  | { kind: "quick" }
  | { kind: "approvals" }
  | { kind: "mission-control" };

export const LAUNCH_EVENT = "bridge-launch";
export const LAUNCH_REQUEST_EVENT = "bridge-launch-request";

/** The shell's payload is still input: accept only the shapes above. */
export function parseLaunch(raw: unknown): LaunchRequest | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  switch (value.kind) {
    case "open":
      return typeof value.sessionId === "string" && value.sessionId ? { kind: "open", sessionId: value.sessionId } : null;
    case "new-chat":
      return { kind: "new-chat", project: typeof value.project === "string" && value.project ? value.project : null };
    case "quick":
    case "approvals":
    case "mission-control":
      return { kind: value.kind };
    default:
      return null;
  }
}

/** Where `--approvals` lands: the first session waiting on you, if any. */
export function approvalTarget(sessions: readonly { id: string; status: string }[]): string | null {
  return sessions.find(session => session.status === "waiting")?.id ?? null;
}
