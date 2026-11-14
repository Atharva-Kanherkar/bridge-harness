/**
 * Trails in mock mode: a few demo trails over the demo chats, and the same
 * membership rules the core enforces (one trail per record, a removal is
 * remembered), so `bun run dev` and component tests exercise the real flows.
 * The core derives timelines from real records; here they are written out.
 */

import type {
  AddTrailMemberResult, CarryTrailBriefResult, CreateTrailResult, ListTrailsResult, PreviewTrailBriefResult,
  ReadTrailResult, RemoveTrailMemberResult, TrailBriefItem, TrailDetail, TrailEvent, TrailMember, TrailMemberRef,
  TrailSummary, UpdateTrailResult,
} from "./protocol/generated/protocol";
import type { TrailsChangedPayload } from "./trails";

type MockTrail = { detail: TrailDetail; brief: TrailBriefItem[] };

const listeners = new Set<(payload: TrailsChangedPayload) => void>();
let trails: MockTrail[] | null = null;
const dismissed = new Set<string>();

function iso(day: number, hour: number, minute = 0): string {
  return new Date(Date.UTC(2026, 10, day, hour, minute)).toISOString();
}

function member(kind: TrailMember["kind"], ref: string, addedBy: TrailMember["addedBy"], addedAt: string): TrailMember {
  return { kind, ref, addedBy, addedAt };
}

function row(member: TrailMember, event: Omit<TrailEvent, "member" | "addedBy" | "id"> & { id?: string }): TrailEvent {
  return { id: event.id ?? `${member.kind}:${member.ref}`, member: { kind: member.kind, ref: member.ref }, addedBy: member.addedBy, ...event };
}

function summary(id: string, title: string, fields: Partial<TrailSummary>): TrailSummary {
  return {
    id, title, workspaceId: "demo-1", state: "inProgress", stage: 2, harnesses: [], sessionIds: [], memberCount: 0,
    createdAt: iso(9, 9), updatedAt: iso(11, 8, 44), ...fields,
  };
}

/** The demo trails' project repository, for `#n` in mock mode. */
const MOCK_REPOSITORY = "github.com/acme/bridge";
const ISSUE_FORMS = "Paste the issue's link, or owner/repo#number";

/** The core's `parse_reference`, for mock mode: a link, `owner/repo#n`, or `#n` with a project. */
export function mockIssueUrl(raw: string, hasProject: boolean): string {
  const text = raw.trim();
  const url = /^(?:https?:\/\/)?([^/\s]+\.[^/\s]+)\/([^/\s]+)\/([^/\s#?]+)\/issues\/(\d+)\/?(?:[?#].*)?$/i.exec(text);
  if (url && Number(url[4]) > 0) return `https://${url[1].toLowerCase()}/${url[2]}/${url[3]}/issues/${Number(url[4])}`;
  if (text.includes("/pull/")) throw new Error(`That link is a pull request, not an issue. ${ISSUE_FORMS}.`);
  const short = /^([^/\s#]+)\/([^/\s#]+)#(\d+)$/.exec(text);
  if (short && Number(short[3]) > 0) return `https://github.com/${short[1]}/${short[2]}/issues/${Number(short[3])}`;
  const bare = /^#?(\d+)$/.exec(text);
  if (bare && Number(bare[1]) > 0) {
    if (!hasProject) throw new Error(`This trail has no project to read ${text.startsWith("#") ? text : `#${text}`} from. ${ISSUE_FORMS}.`);
    return `https://${MOCK_REPOSITORY}/issues/${Number(bare[1])}`;
  }
  throw new Error(`${ISSUE_FORMS}.`);
}

function seed(): MockTrail[] {
  const issue = member("issue", "https://github.com/acme/bridge/issues/1004", "person", iso(9, 9));
  const plan = member("note", "note-plan", "person", iso(9, 10, 1));
  const design = member("session", "session-1w", "person", iso(9, 9, 20));
  const build = member("session", "session-1", "auto", iso(10, 11, 34));
  const pr = member("pullRequest", "https://github.com/acme/bridge/pull/1012", "auto", iso(10, 12, 50));
  const dm = member("outgoing", "outgoing-trail-1", "auto", iso(10, 12, 58));
  const archive: TrailDetail = {
    summary: summary("trail-archive", "Archive a chat from the sidebar", {
      state: "needsYou", needsYouReason: "vitest is failing on #1012", stage: 3,
      lastEvent: { title: "Checks on #1012", at: iso(11, 8, 44) },
      harnesses: [{ harness: "claude", chats: 1 }, { harness: "codex", chats: 1 }],
      sessionIds: ["session-1w", "session-1"], memberCount: 6,
    }),
    members: [issue, design, plan, build, pr, dm],
    branch: "feat/archive-chat",
    events: [
      row(issue, { kind: "issue", at: iso(8, 16, 12), title: "#1004 Archive a chat without losing its history", detail: "open · 2 comments", url: issue.ref, tone: "plain" }),
      row(design, { kind: "chat", at: iso(9, 9, 20), title: "Design the archive action", detail: "Claude · fable", harness: "claude", tone: "plain" }),
      row(plan, { kind: "note", at: iso(9, 10, 1), title: "Plan: archive action per chat", detail: "last edited by you", tone: "plain" }),
      row(build, { kind: "chat", at: iso(10, 11, 34), title: "Implement the archive row action", detail: "Codex · gpt-5.6-luna", harness: "codex", tone: "plain" }),
      row(pr, { kind: "pullRequest", at: iso(10, 12, 50), title: "#1012 feat(sidebar): archive a chat from its row", detail: "open", url: pr.ref, tone: "plain" }),
      row(dm, { kind: "message", at: iso(10, 12, 58), title: "Message to Slack · @mika", detail: "copied to send", excerpt: "Could you review #1012 today? It adds the archive action we discussed. The undo toast is the part I am least sure about.", tone: "plain" }),
      row(pr, { id: `pullRequest:${pr.ref}#checks`, kind: "checks", at: iso(11, 8, 44), title: "Checks on #1012", detail: "11 of 12 passing · 1 failing", url: pr.ref, tone: "danger" }),
    ],
    open: [
      { kind: "failingCheck", label: "vitest failing on #1012", member: { kind: "pullRequest", ref: pr.ref }, url: pr.ref },
    ],
  };
  const reply = member("session", "session-2", "person", iso(12, 15));
  const draft = member("outgoing", "outgoing-trail-2", "auto", iso(13, 9, 10));
  const outgoing: TrailDetail = {
    summary: summary("trail-reply", "Outgoing: reply watch", {
      workspaceId: "demo-2", state: "needsYou", needsYouReason: "A draft to Slack · #dev is not sent", stage: 2,
      lastEvent: { title: "Message to Slack · #dev", at: iso(13, 9, 10) },
      harnesses: [{ harness: "codex", chats: 1 }], sessionIds: ["session-2"], memberCount: 2, updatedAt: iso(13, 9, 10),
    }),
    members: [reply, draft],
    events: [
      row(reply, { kind: "chat", at: iso(12, 15), title: "Orchestrator", detail: "Codex", harness: "codex", tone: "plain" }),
      row(draft, { kind: "message", at: iso(13, 9, 10), title: "Message to Slack · #dev", detail: "draft, not sent yet", excerpt: "Reply watch is ready for a look: replies to a sent draft now land back in the chat that wrote it.", tone: "warning" }),
    ],
    open: [{ kind: "unsentDraft", label: "Send or discard the draft to Slack · #dev", member: { kind: "outgoing", ref: draft.ref } }],
  };
  const shippedPr = member("pullRequest", "https://github.com/acme/bridge/pull/979", "auto", iso(13, 8));
  const stage = member("session", "session-2w", "person", iso(12, 20));
  const shipped: TrailDetail = {
    summary: summary("trail-stage", "Mission Control stage and queue", {
      state: "shipped", stage: 4, lastEvent: { title: "#979 Mission Control stage and queue", at: iso(13, 8) },
      harnesses: [{ harness: "opencode", chats: 1 }], sessionIds: ["session-2w"], memberCount: 2, updatedAt: iso(13, 9),
    }),
    members: [stage, shippedPr],
    events: [
      row(stage, { kind: "chat", at: iso(12, 20), title: "Docs · fast", detail: "OpenCode", harness: "opencode", tone: "plain" }),
      row(shippedPr, { kind: "pullRequest", at: iso(13, 8), title: "#979 Mission Control stage and queue", detail: "merged", url: shippedPr.ref, tone: "success" }),
    ],
    open: [],
  };
  return [
    {
      detail: archive,
      brief: [
        { id: `issue:${issue.ref}`, label: "Issue #1004 Archive a chat without losing its history", tokens: 420, defaultOn: true },
        { id: "note:note-plan", label: "Note “Plan: archive action per chat”", tokens: 612, defaultOn: true },
        { id: `pr:${pr.ref}`, label: "PR #1012 feat(sidebar): archive a chat from its row: 1 failing", tokens: 180, defaultOn: true },
        { id: "chat:session-1w", label: "Chat “Design the archive action” (Claude)", tokens: 1840, defaultOn: true },
        { id: "chat:session-1", label: "Chat “Implement the archive row action” (Codex)", tokens: 2950, defaultOn: true },
        { id: "message:outgoing-trail-1", label: "Message to Slack · @mika", tokens: 64, defaultOn: true },
      ],
    },
    { detail: outgoing, brief: [{ id: "chat:session-2", label: "Chat “Orchestrator” (Codex)", tokens: 2210, defaultOn: true }, { id: "message:outgoing-trail-2", label: "Message to Slack · #dev", tokens: 48, defaultOn: true }] },
    { detail: shipped, brief: [{ id: `pr:${shippedPr.ref}`, label: "PR #979 Mission Control stage and queue", tokens: 96, defaultOn: true }] },
  ];
}

function store(): MockTrail[] {
  if (!trails) trails = seed();
  return trails;
}

/** Back to the seeded demo, for tests. */
export function resetMockTrails(): void {
  trails = null;
  dismissed.clear();
}

function find(id: string): MockTrail {
  const trail = store().find(item => item.detail.summary.id === id);
  if (!trail) throw new Error("That trail no longer exists.");
  return trail;
}

function emit(payload: TrailsChangedPayload): void {
  for (const listener of listeners) listener(payload);
}

function touch(trail: MockTrail): void {
  trail.detail.summary = { ...trail.detail.summary, updatedAt: new Date().toISOString(), memberCount: trail.detail.members.length, sessionIds: trail.detail.members.filter(m => m.kind === "session").map(m => m.ref) };
}

const RANK = { needsYou: 0, inProgress: 1, shipped: 2 } as const;

export function mockListTrails(includeArchived = false): ListTrailsResult {
  const list = store().map(trail => trail.detail.summary).filter(item => includeArchived || !item.archivedAt);
  list.sort((a, b) => RANK[a.state] - RANK[b.state] || b.updatedAt.localeCompare(a.updatedAt));
  return { trails: list };
}

export function mockReadTrail(trailId: string): ReadTrailResult {
  return { trail: structuredClone(find(trailId).detail) };
}

export function mockCreateTrail(title: string, workspaceId?: string | null, sessionId?: string | null): CreateTrailResult {
  const clean = title.split(/\s+/).filter(Boolean).join(" ");
  if (!clean) throw new Error("A trail needs a title.");
  if (sessionId) {
    const owner = store().find(trail => trail.detail.members.some(m => m.kind === "session" && m.ref === sessionId));
    if (owner) throw new Error(`This is already on the trail “${owner.detail.summary.title}”. Remove it there first.`);
  }
  const now = new Date().toISOString();
  const members: TrailMember[] = sessionId ? [member("session", sessionId, "person", now)] : [];
  const id = `trail-${crypto.randomUUID()}`;
  const detail: TrailDetail = {
    summary: summary(id, clean, { workspaceId: workspaceId ?? undefined, stage: sessionId ? 2 : 0, sessionIds: sessionId ? [sessionId] : [], memberCount: members.length, createdAt: now, updatedAt: now }),
    members,
    events: sessionId ? [row(members[0], { kind: "chat", at: now, title: "This chat", tone: "plain" })] : [],
    open: [],
  };
  store().unshift({ detail, brief: [] });
  emit({ trailId: id, added: [] });
  return { trail: detail.summary };
}

export function mockUpdateTrail(trailId: string, title?: string | null, archived?: boolean | null): UpdateTrailResult {
  const trail = find(trailId);
  if (title != null) {
    const clean = title.split(/\s+/).filter(Boolean).join(" ");
    if (!clean) throw new Error("A trail needs a title.");
    trail.detail.summary = { ...trail.detail.summary, title: clean };
  }
  if (archived != null) trail.detail.summary = { ...trail.detail.summary, archivedAt: archived ? new Date().toISOString() : undefined };
  touch(trail);
  emit({ trailId, added: [] });
  return { trail: trail.detail.summary };
}

const key = (trailId: string, ref: TrailMemberRef) => `${trailId}|${ref.kind}|${ref.ref}`;

export function mockAddTrailMember(trailId: string, given: TrailMemberRef): AddTrailMemberResult {
  const trail = find(trailId);
  const ref = given.kind === "issue" ? { kind: "issue" as const, ref: mockIssueUrl(given.ref, Boolean(trail.detail.summary.workspaceId)) } : given;
  const owner = store().find(item => item.detail.members.some(m => m.kind === ref.kind && m.ref === ref.ref));
  if (owner && owner !== trail) throw new Error(`This is already on the trail “${owner.detail.summary.title}”. Remove it there first.`);
  if (!owner) {
    const added = member(ref.kind, ref.ref, "person", new Date().toISOString());
    trail.detail.members.push(added);
    if (ref.kind === "issue") {
      const number = ref.ref.split("/").pop();
      // The core orders the timeline by time; so does the mock.
      trail.detail.events.push(row(added, { kind: "issue", at: added.addedAt, title: `#${number} Issue read in mock mode`, detail: "open", url: ref.ref, tone: "plain" }));
      trail.detail.events.sort((left, right) => Date.parse(left.at) - Date.parse(right.at));
      if (trail.detail.summary.stage === 0) trail.detail.summary = { ...trail.detail.summary, stage: 1 };
    } else {
      trail.detail.events.push(row(added, { kind: ref.kind === "session" ? "chat" : ref.kind === "outgoing" ? "message" : ref.kind, at: added.addedAt, title: ref.kind === "session" ? "A chat" : ref.ref, tone: "plain" }));
    }
    dismissed.delete(key(trailId, ref));
    touch(trail);
  }
  emit({ trailId, added: [] });
  return { trail: trail.detail.summary };
}

export function mockRemoveTrailMember(trailId: string, ref: TrailMemberRef): RemoveTrailMemberResult {
  const trail = find(trailId);
  trail.detail.members = trail.detail.members.filter(m => !(m.kind === ref.kind && m.ref === ref.ref));
  trail.detail.events = trail.detail.events.filter(e => !(e.member.kind === ref.kind && e.member.ref === ref.ref));
  trail.detail.open = trail.detail.open.filter(o => !(o.member.kind === ref.kind && o.member.ref === ref.ref));
  dismissed.add(key(trailId, ref));
  touch(trail);
  emit({ trailId, added: [] });
  return { trail: trail.detail.summary };
}

/** A new chat on the default 128k window carries at most this many tokens. */
export const MOCK_BRIEF_BUDGET = 14_400;

export function mockPreviewTrailBrief(trailId: string): PreviewTrailBriefResult {
  const items = find(trailId).brief.filter(item => !dismissed.has(key(trailId, refOf(item.id))));
  return { items, totalTokens: items.filter(item => item.defaultOn).reduce((sum, item) => sum + item.tokens, 0), budgetTokens: MOCK_BRIEF_BUDGET };
}

function refOf(id: string): TrailMemberRef {
  const [kind, ...rest] = id.split(":");
  const ref = rest.join(":");
  return { kind: kind === "chat" ? "session" : kind === "pr" ? "pullRequest" : kind === "message" ? "outgoing" : kind === "issue" ? "issue" : "note", ref };
}

export function mockCarryTrailBrief(trailId: string, sessionId: string, items: string[]): CarryTrailBriefResult {
  const trail = find(trailId);
  const known = new Set(trail.brief.map(item => item.id));
  const unknown = items.find(item => !known.has(item));
  if (unknown) throw new Error(`The trail has no brief item “${unknown}”. Reload the trail and try again.`);
  const tokens = trail.brief.filter(item => items.includes(item.id)).reduce((sum, item) => sum + item.tokens, 0);
  if (tokens > MOCK_BRIEF_BUDGET) throw new Error(`These items need about ${tokens} tokens; this chat can start with at most ${MOCK_BRIEF_BUDGET}. Untick some and try again.`);
  const added = member("session", sessionId, "person", new Date().toISOString());
  if (!trail.detail.members.some(m => m.kind === "session" && m.ref === sessionId)) {
    trail.detail.members.push(added);
    trail.detail.events.push(row(added, { kind: "chat", at: added.addedAt, title: "New chat on this trail", tone: "plain" }));
  }
  touch(trail);
  emit({ trailId, added: [] });
  return { carried: items.length, trail: trail.detail.summary };
}

export function onMockTrailsChanged(handler: (payload: TrailsChangedPayload) => void): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

/** Simulate the core's automatic sweep announcing an addition (demo toast). */
export function mockAnnounceTrailAddition(): void {
  const trail = store()[0];
  emit({ trailId: trail.detail.summary.id, added: [{ trailId: trail.detail.summary.id, trailTitle: trail.detail.summary.title, member: { kind: "pullRequest", ref: "https://github.com/acme/bridge/pull/1012" }, label: "PR #1012 feat(sidebar): archive a chat from its row" }] });
}
