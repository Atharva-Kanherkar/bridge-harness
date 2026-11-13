/**
 * Drafted messages in mock mode: one demo draft in the demo chat, and the
 * same revision rules the core enforces, so the card behaves as it will
 * against a real backend. The core owns the real style rules; this mock
 * only knows em dashes, which is enough to exercise Fix.
 */

import type { CopyOutgoingResult, DiscardOutgoingResult, ListOutgoingResult, OutgoingLintFinding, OutgoingMessage, ReadOutgoingAttachmentResult, SendOutgoingResult, UpdateOutgoingResult } from "./protocol/generated/protocol";

/** A small greyscale error-rate chart, standing in for a screenshot. */
export const MOCK_PNG_BASE64 = "iVBORw0KGgoAAAANSUhEUgAAAKAAAABgCAAAAAC/TCZFAAABwklEQVR42u3Y23GEMAwFUBV2+y9ENaSAhGQX1mCD9QJlRvrarztHss2sTV/JiwpYwAIWsIAFLGABC1jAAhawgI8DsVRiIF6VFQjohTcBeam0wJdvISYF8lopgUgP5NxApAdybiDSA7mAtuICFrCABSxgAQtYwAL+E6D+gWceCAPQ8MAjAEINtDzwSIBQArE98GB3K7Oh2Sik1rcjOoyV2bbIdLy9riB8lhdQKqTO7bqBaTf3ECjMos7ldSNajt8JEFIg0EuB8QPRbb17EK+BfF2OQJZsauoP0Es4iMb8+aPJAeoemMfR00IJEI7AlngOnPRpRojJzuEERAzwVEiTR0Q1wunocyBz1AjhkCwCCkcIj95J4pMC+XagSAhZ77gfyB7RJExB0ACH0UKgYIRC3yg6DAg5EB7AaaHYN4iOAuI5IKJ8fSFxxAjxKBAD0vrHDkpfN5pc+sS+WFdOQET5eiMkhz4/SCZed4Rk79NGuhghyNynp6+zOmTt09d3XB0yxjj7DqvDWiCCfO0I1cC/9x/rkb0ULunEemGIr9k+rAdylG8Twgb8CeKg+t3h7/aJ81WzfTICm+2TErgQ37+SArf6BsYsowlIg1IcAAAAAElFTkSuQmCC";

const EM_DASH = "—";

const listeners = new Set<(payload: { sessionId: string; id: string }) => void>();

export const MOCK_OUTGOING_ID = "outgoing-demo-1";

function demo(): OutgoingMessage {
  const body = "Deploy to staging is done, all checks green.\n\n| service | status |\n|---|---|\n| api | ok |\n| worker | ok |\n\n```bash\nbun run deploy --env staging\n```\n\nI'll watch the error rate for an hour before promoting.";
  return {
    id: MOCK_OUTGOING_ID,
    sessionId: "session-1",
    destination: { kind: "slackThread", channel: "#dev", thread: "https://acme.slack.com/archives/C01/p1730000000" },
    label: "Slack · #dev · thread",
    format: "slackMarkdown",
    body,
    rendered: body,
    lint: [
      { rule: "no_em_dash", message: "Em dash replaced with a comma", line: 1, fixable: true, fixed: true },
    ],
    attachments: [{ path: "/Users/demo/Desktop/error-rate.png", name: "error-rate.png", mime: "image/png", bytes: 507 }],
    state: "draft",
    revision: 1,
    reason: "tell #dev the staging deploy finished",
    createdAt: "2026-11-08T09:00:00Z",
    updatedAt: "2026-11-08T09:00:00Z",
  };
}

let items: OutgoingMessage[] = [demo()];
const approvals = new Map<string, { token: string; revision: number }>();

/** Back to the single demo draft. Tests call this between cases. */
export function resetMockOutgoing(seed: OutgoingMessage[] = [demo()]) {
  items = seed.map(item => structuredClone(item));
  approvals.clear();
}

function lint(body: string): OutgoingLintFinding[] {
  return body.split("\n").flatMap((line, index) => line.includes(EM_DASH)
    ? [{ rule: "no_em_dash", message: "Em dash", line: index + 1, fixable: true, fixed: false }]
    : []);
}

function find(id: string, revision?: number): OutgoingMessage {
  const item = items.find(candidate => candidate.id === id);
  if (!item) throw new Error("that draft no longer exists");
  if (item.state === "sent" || item.state === "sending") throw new Error("this message is already sending or sent");
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

export async function mockUpdateOutgoing(id: string, revision: number, body?: string, applyFixes = false, attachments?: string[]): Promise<UpdateOutgoingResult> {
  const item = find(id, revision);
  if (body === undefined && !applyFixes && attachments === undefined) throw new Error("nothing to change");
  if (attachments !== undefined) item.attachments = (item.attachments ?? []).filter(attachment => attachments.includes(attachment.path));
  let text = (body ?? item.body).trim();
  if (!text) throw new Error("the message is empty");
  let findings = lint(text);
  if (applyFixes) {
    const fixed = findings.map(finding => ({ ...finding, message: "Em dash replaced with a comma", fixed: true }));
    text = text.replace(/[ \t]*—[ \t]*/g, ", ");
    findings = fixed;
  }
  approvals.delete(id);
  delete item.lastError;
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

export async function mockReadOutgoingAttachment(id: string, index: number): Promise<ReadOutgoingAttachmentResult> {
  const attachment = items.find(candidate => candidate.id === id)?.attachments?.[index];
  if (!attachment) throw new Error("that draft has no such image");
  return { mime: "image/png", dataBase64: MOCK_PNG_BASE64 };
}

export async function mockDiscardOutgoing(id: string): Promise<DiscardOutgoingResult> {
  const item = items.find(candidate => candidate.id === id);
  if (!item) throw new Error("that draft no longer exists");
  if (item.state === "sent" || item.state === "sending") throw new Error("this message is already sending or sent");
  approvals.delete(id);
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

/** The mock never touches a provider; it exercises the real two-call UI. */
export async function mockSendOutgoing(id: string, revision: number, approved?: boolean, approvalToken?: string): Promise<SendOutgoingResult> {
  if (approved === false) {
    if (approvals.get(id)?.token === approvalToken) approvals.delete(id);
    return { kind: "refused", detail: "Sending cancelled" };
  }
  let item: OutgoingMessage;
  try { item = find(id, revision); } catch (error) { return { kind: "refused", detail: String(error) }; }
  if (item.destination.kind === "clipboard") return { kind: "refused", detail: "this destination is copy only" };
  if (approved !== true) {
    const token = crypto.randomUUID();
    approvals.set(id, { token, revision });
    return { kind: "approvalRequired", effect: `Send to ${item.label}:\n\n${item.rendered}${(item.attachments ?? []).map(image => `\nImage: ${image.name} (${image.bytes} bytes)`).join("")}`, rendered: item.rendered, approvalToken: token, item: structuredClone(item) };
  }
  const pending = approvals.get(id);
  if (!pending || pending.token !== approvalToken || pending.revision !== revision) return { kind: "refused", detail: "review this message before sending; the approval is missing or stale" };
  approvals.delete(id);
  item.state = "sending";
  changed(item);
  await new Promise(resolve => window.setTimeout(resolve, 100));
  item.state = "sent";
  item.remoteRef = "1794571200.000001";
  item.permalink = item.destination.kind.startsWith("github") ? "https://github.com/example/project/issues/1#issuecomment-42" : "https://acme.slack.com/archives/C01/p1794571200000001";
  item.sentAt = new Date().toISOString();
  delete item.lastError;
  changed(item);
  return { kind: "sent", item: structuredClone(item) };
}
