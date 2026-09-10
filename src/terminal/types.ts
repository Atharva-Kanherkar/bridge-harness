export type { TerminalRecord, TerminalSnapshot, TerminalWorkspace, CreateTerminalParams } from "../protocol/generated/protocol";
export type TerminalFrame = { workspaceId: string; terminalId: string; generation: string; sequence: number; data: string; rows?: number | null; cols?: number | null; status?: string | null };
