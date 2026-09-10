import type { CreateTerminalParams, TerminalRecord, TerminalSnapshot, TerminalWorkspace } from "./types";

// Browser preview data is kept separate from the native process implementation.
const stores = new Map<string, TerminalWorkspace>();
export function mockTerminalWorkspace(workspaceId: string): TerminalWorkspace {
  let store = stores.get(workspaceId);
  if (!store) { store = { terminals: [], layout: null }; stores.set(workspaceId, store); }
  return structuredClone(store);
}
export function mockCreateTerminal(params: CreateTerminalParams): TerminalRecord {
  const workspace = mockTerminalWorkspace(params.workspaceId);
  const existing = workspace.terminals.find(t => t.terminalId === params.terminalId);
  if (existing && !params.restart) return existing;
  const record: TerminalRecord = { workspaceId: params.workspaceId, terminalId: params.terminalId, generation: crypto.randomUUID(), title: existing?.title ?? params.agentId ?? "Shell", agentId: params.agentId ?? existing?.agentId ?? null, cwd: params.cwd ?? existing?.cwd ?? "/workspace", status: "running", rows: 32, cols: 120, createdAt: new Date().toISOString(), exitCode: null, historyTruncated: false };
  workspace.terminals = [...workspace.terminals.filter(t => t.terminalId !== record.terminalId), record];
  stores.set(params.workspaceId, workspace);
  return record;
}
export function mockSnapshot(workspaceId: string, terminalId: string): TerminalSnapshot {
  const record = mockTerminalWorkspace(workspaceId).terminals.find(t => t.terminalId === terminalId);
  if (!record) throw new Error("Terminal not found");
  return { record, sequence: 0, ansi: `\x1b[90mBrowser preview · open Bridge desktop for a live terminal.\x1b[0m\r\n\r\n${record.agentId ? `${record.agentId} › ` : "$ "}` };
}
export function mockSaveLayout(workspaceId: string, layout: unknown) { stores.set(workspaceId, { ...mockTerminalWorkspace(workspaceId), layout }); }
export function mockRenameTerminal(workspaceId: string, terminalId: string, title: string): TerminalRecord {
  const store = mockTerminalWorkspace(workspaceId);
  const record = store.terminals.find(t => t.terminalId === terminalId);
  if (!record) throw new Error("Terminal not found");
  record.title = title;
  stores.set(workspaceId, store);
  return record;
}
export function mockCloseTerminal(workspaceId: string, terminalId: string) { const store = mockTerminalWorkspace(workspaceId); store.terminals = store.terminals.filter(t => t.terminalId !== terminalId); stores.set(workspaceId, store); }
