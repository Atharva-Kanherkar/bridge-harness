import type { GithubRepository } from "./protocol/generated/protocol";

// Which view of the GitHub pane a github.com URL names. The pane renders pull
// requests, issues and a repository overview for one repository — the one the
// workspace's Git remote resolves to — so those are exactly the shapes a link
// can be routed into. Every other path (a commit, a blob, a release, an
// Actions run, a discussion, a profile) has no inline view and keeps going to
// the browser, which is why parsing returns null rather than a best guess.

export type PullRequestView = "conversation" | "changes" | "checks";

export type GithubLinkView =
  | { kind: "pull"; number: number; tab: PullRequestView }
  | { kind: "issue"; number: number }
  | { kind: "pulls" }
  | { kind: "issues" }
  | { kind: "repository" };

export type GithubLink = {
  /** Lowercased, so a GitHub Enterprise host compares like github.com does. */
  host: string;
  owner: string;
  name: string;
  view: GithubLinkView;
};

/** The sub-paths of a pull request that the detail view has a tab for.
 * Anything else under `/pull/<n>` (a commit, a comparison) still names that
 * pull request, so it lands on the conversation rather than nowhere. */
const PULL_REQUEST_TABS: Record<string, PullRequestView> = {
  files: "changes",
  checks: "checks",
};

/** GitHub numbers issues and pull requests from 1, and a leading zero or a
 * sign is not something it ever links. Parse strictly rather than letting
 * `Number()` accept `1.5`, `1e3`, or whitespace. */
function pathNumber(segment: string | undefined): number | null {
  if (!segment || !/^[1-9]\d*$/.test(segment)) return null;
  return Number(segment);
}

function repositoryName(segment: string): string {
  return segment.endsWith(".git") ? segment.slice(0, -".git".length) : segment;
}

/**
 * Parse a URL into the pane view that would render it, or null when the pane
 * has no view for it. The host is carried but never checked against an
 * allowlist here: whether a link can be shown inline is decided by
 * `githubLinkMatchesRepository` against the repository this workspace
 * actually resolves to, which is what makes GitHub Enterprise work and what
 * keeps a look-alike host from routing anywhere.
 */
export function parseGithubLink(raw: string): GithubLink | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;

  const host = url.hostname.toLowerCase();
  if (!host) return null;

  let segments: string[];
  try {
    segments = url.pathname.split("/").filter(Boolean).map(segment => decodeURIComponent(segment));
  } catch {
    // A malformed percent-escape is not a route; let it go to the browser.
    return null;
  }
  const [owner, repo, section, fourth] = segments;
  if (!owner || !repo) return null;

  const link = (view: GithubLinkView): GithubLink => ({ host, owner, name: repositoryName(repo), view });

  if (section === undefined) return link({ kind: "repository" });

  if (section === "pull") {
    const number = pathNumber(fourth);
    if (number === null) return null;
    return link({ kind: "pull", number, tab: PULL_REQUEST_TABS[segments[4] ?? ""] ?? "conversation" });
  }

  if (section === "issues") {
    if (fourth === undefined) return link({ kind: "issues" });
    const number = pathNumber(fourth);
    // `/issues/new` and the like are pages the pane cannot open.
    if (number === null || segments.length > 4) return null;
    return link({ kind: "issue", number });
  }

  if (section === "pulls" && fourth === undefined) return link({ kind: "pulls" });

  return null;
}

function sameToken(left: string, right: string): boolean {
  return left.toLowerCase() === right.toLowerCase();
}

/**
 * Whether the pane could show this link at all. Every `github/*` read is
 * scoped to a workspace and resolves its repository server-side from that
 * workspace's Git remote — no call may name a repository — so a link to any
 * other repository has no inline representation and belongs in the browser.
 */
export function githubLinkMatchesRepository(
  link: GithubLink,
  repository: GithubRepository | null | undefined,
): boolean {
  if (!repository) return false;
  return sameToken(link.host, repository.host)
    && sameToken(link.owner, repository.owner)
    && sameToken(link.name, repositoryName(repository.name));
}
