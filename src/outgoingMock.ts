/**
 * Drafted messages in mock mode: one demo draft in the demo chat, and the
 * same revision rules the core enforces, so the card behaves as it will
 * against a real backend. The core owns the real style rules; this mock
 * only knows em dashes, which is enough to exercise Fix.
 */

import type { CopyOutgoingResult, DiscardOutgoingResult, ListOutgoingResult, OutgoingLintFinding, OutgoingMessage, UpdateOutgoingResult } from "./protocol/generated/protocol";

const EM_DASH = "—";

const listeners = new Set<(payload: { sessionId: string; id: string }) => void>();

export const MOCK_OUTGOING_ID = "outgoing-demo-1";

function demo(): OutgoingMessage {
  const body = "Deploy to staging is done, all checks green.\n\n| service | status |\n|---|---|\n| api | ok |\n\nI'll watch the error rate for an hour before promoting.";
  return {
    id: MOCK_OUTGOING_ID,
    sessionId: "session-1",
    destination: { kind: "slackThread", channel: "#dev", thread: "https://acme.slack.com/archives/C01/p1730000000" },
    label: "Slack · #dev · thread",
    format: "slack",
    body,
    rendered: body,
    lint: [
      { rule: "no_em_dash", message: "Em dash replaced with a comma", line: 1, fixable: true, fixed: true },
      { rule: "slack_table", message: "Table: Slack shows it as raw text", line: 3, fixable: false, fixed: false },
    ],
    state: "draft",
    revision: 1,
    reason: "tell #dev the staging deploy finished",
    createdAt: "2026-11-08T09:00:00Z",
    updatedAt: "2026-11-08T09:00:00Z",
  };
}

let items: OutgoingMessage[] = [demo()];

/** Back to the single demo draft. Tests call this between cases. */
export function resetMockOutgoing(seed: OutgoingMessage[] = [demo()]) {
  items = seed.map(item => structuredClone(item));
}

function lint(body: string): OutgoingLintFinding[] {
  return body.split("\n").flatMap((line, index) => line.includes(EM_DASH)
    ? [{ rule: "no_em_dash", message: "Em dash", line: index + 1, fixable: true, fixed: false }]
    : []);
}

function find(id: string, revision?: number): OutgoingMessage {
  const item = items.find(candidate => candidate.id === id);
  if (!item) throw new Error("that draft no longer exists");
  if (item.state === "discarded") throw new Error("this draft was discarded");
  if (revision !== undefined && revision !== item.revision) throw new Error("this draft changed since you opened it; reload it and try again");
  return item;
}

function changed(item: OutgoingMessage) {
  item.updatedAt = new Date().toISOString();
  for (const listener of listeners) listener({ sessionId: item.sessionId, id: item.id });
}

export function mockListOutgoing(sessionId: string): ListOutgoingResult {
  return { items: items.filter(item => item.sessionId === sessionId).map(item => structuredClone(item)) };
}

export async function mockUpdateOutgoing(id: string, revision: number, body?: string, applyFixes = false): Promise<UpdateOutgoingResult> {
  const item = find(id, revision);
  if (body === undefined && !applyFixes) throw new Error("nothing to change");
  let text = (body ?? item.body).trim();
  if (!text) throw new Error("the message is empty");
  let findings = lint(text);
  if (applyFixes) {
    const fixed = findings.map(finding => ({ ...finding, message: "Em dash replaced with a comma", fixed: true }));
    text = text.replace(/[ \t]*—[ \t]*/g, ", ");
    findings = fixed;
  }
  Object.assign(item, { body: text, rendered: text, lint: findings, state: "draft", revision: item.revision + 1 });
  changed(item);
  return { item: structuredClone(item) };
}

export async function mockCopyOutgoing(id: string, revision: number): Promise<CopyOutgoingResult> {
  const item = find(id, revision);
  item.state = "copied";
  changed(item);
  return { text: item.rendered, item: structuredClone(item) };
}

export async function mockDiscardOutgoing(id: string): Promise<DiscardOutgoingResult> {
  const item = items.find(candidate => candidate.id === id);
  if (!item) throw new Error("that draft no longer exists");
  if (item.state !== "discarded") {
    item.state = "discarded";
    changed(item);
  }
  return { item: structuredClone(item) };
}

export function onMockOutgoingChanged(handler: (payload: { sessionId: string; id: string }) => void): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}
