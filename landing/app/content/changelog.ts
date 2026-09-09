export type ReleaseEntry = {
  version: string;
  date: string;
  headline: string;
  bullets: string[];
};

export const releases: ReleaseEntry[] = [
  {
    version: "0.5.5",
    date: "September 8, 2026",
    headline: "The Span app icon",
    bullets: [
      "A mint deck over two off-white supports on a dark tile, regenerated for every platform size including each macOS ICNS entry.",
      "Release checks now catch missing icon colors and invisible exports before a build ships.",
      "Keeps the startup, window lifetime, daemon ownership, and shutdown fixes from 0.5.4.",
    ],
  },
  {
    version: "0.5.4",
    date: "September 8, 2026",
    headline: "Streaming feedback and bounded repair loops",
    bullets: [
      "Sending a message gives immediate feedback, and streaming and transcript updates land more smoothly.",
      "Worker-result repair loops are bounded, and a reused worker starts from a fresh budget.",
      "Better model-switch handoffs, clearer compaction ownership, and steadier context checkpoints.",
      "Clean shutdown now also retires detached model-switch summary processes.",
    ],
  },
  {
    version: "0.5.3",
    date: "September 7, 2026",
    headline: "Shutdown no longer strands a finished session",
    bullets: [
      "Quitting with an idle provider could leave stale process ownership in the database, so a chat that had succeeded appeared failed at the next launch.",
      "Normal shutdown now records a recoverable stopped state and preserves both the conversation and the provider resume identifier.",
      "Completed, cancelled, and failed outcomes are unchanged.",
    ],
  },
  {
    version: "0.5.2",
    date: "September 7, 2026",
    headline: "macOS startup and window lifecycle",
    bullets: [
      "Startup errors report their actual cause instead of aborting through the native launch callback.",
      "Concurrent copies of the app can no longer start competing desktop hosts or kill another build's daemon.",
      "Long socket paths fail early with an actionable error.",
      "Packaging verifies the icon, sidecars, signatures, entitlements, notarization ticket, and exact disk image contents.",
    ],
  },
  {
    version: "0.5.1",
    date: "September 4, 2026",
    headline: "The notarized build launches",
    bullets: [
      "WebKit JIT entitlements are granted under the Hardened Runtime.",
      "Window chrome catches Objective-C exceptions instead of aborting the process.",
    ],
  },
  {
    version: "0.5.0",
    date: "September 4, 2026",
    headline: "First public macOS disk image",
    bullets: [
      "Developer ID signed and notarized for macOS 12 or later on Apple Silicon.",
      "The Codex, Claude Code, and OpenCode command-line tools stay optional.",
    ],
  },
];
