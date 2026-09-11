export type DocEntry = {
  slug: string;
  file: string;
  title: string;
  summary: string;
};

export type DocGroup = {
  title: string;
  entries: DocEntry[];
};

export const docGroups: DocGroup[] = [
  {
    title: "Core concepts",
    entries: [
      {
        slug: "session-forest",
        file: "session-forest.md",
        title: "Session forest",
        summary: "Immutable entries, branch selection, and the stamp that surfaces conversation and file divergence.",
      },
      {
        slug: "local-history",
        file: "local-history.md",
        title: "Durable local history",
        summary: "What Bridge stores on your machine, and what that store is not.",
      },
      {
        slug: "compaction-and-resume",
        file: "compaction-and-resume.md",
        title: "Compaction and resume",
        summary: "Who owns the context window, and the four things a returning session is allowed to claim.",
      },
      {
        slug: "worktree-lifecycle",
        file: "worktree-lifecycle.md",
        title: "Worktree lifecycle",
        summary: "The inventory of checkouts, the build caches that keep them small, and the retention policy that reclaims them.",
      },
    ],
  },
  {
    title: "Delegation",
    entries: [
      {
        slug: "delegation-policy",
        file: "delegation-policy.md",
        title: "Delegation policy",
        summary: "Typed request envelopes, the six decisions Rust can return, and every gate in between.",
      },
      {
        slug: "adaptive-learning",
        file: "adaptive-learning.md",
        title: "Role profiles and adaptive learning",
        summary: "How routing ranks eligible candidates without ever widening what policy allows.",
      },
      {
        slug: "transcript-behavior-contract",
        file: "transcript-behavior-contract.md",
        title: "Transcript behavior contract",
        summary: "What a reader is entitled to see identically, whichever harness produced the turn.",
      },
    ],
  },
  {
    title: "Runtime",
    entries: [
      {
        slug: "protocol",
        file: "protocol/README.md",
        title: "Protocol",
        summary: "The JSON-RPC contract, its generated artifacts, and the bridge exec one-shot for CI.",
      },
      {
        slug: "managed-agent-runtimes",
        file: "managed-agent-runtimes.md",
        title: "Managed agent runtimes",
        summary: "Installing vendor runtimes as pinned closures, proven by receipt rather than inferred.",
      },
      {
        slug: "authenticated-browser-bridge",
        file: "authenticated-browser-bridge.md",
        title: "Authenticated browser bridge",
        summary: "Supervising one approved tab while the browser stays the credential boundary.",
      },
      {
        slug: "memory-ledger",
        file: "memory-ledger.md",
        title: "Account memory ledger",
        summary: "What Bridge remembers across sessions, and how a memory is recorded.",
      },
    ],
  },
  {
    title: "Operations",
    entries: [
      {
        slug: "macos-release",
        file: "macos-release.md",
        title: "Releasing for macOS",
        summary: "Signing, notarization, and the gates a build passes before an artifact is published.",
      },
    ],
  },
];

export const docEntries = docGroups.flatMap((group) => group.entries);

export function docBySlug(slug: string) {
  return docEntries.find((entry) => entry.slug === slug);
}

export function docByFile(file: string) {
  return docEntries.find((entry) => entry.file === file);
}
