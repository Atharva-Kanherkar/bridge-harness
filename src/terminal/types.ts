export type { TerminalRecord, TerminalSnapshot, TerminalWorkspace, CreateTerminalParams } from "../protocol/generated/protocol";
export type TerminalFrame = { workspaceId: string; terminalId: string; generation: string; sequence: number; data: string; rows?: number | null; cols?: number | null; status?: string | null };

/** Lines of history above the screen, in xterm and in the runtime checkpoint. */
export const TERMINAL_SCROLLBACK = 5000;
