export type ReleaseEntry = {
  version: string;
  date: string;
  headline: string;
  bullets: string[];
};

export const releases: ReleaseEntry[] = [
  {
    version: "0.5.10",
    date: "September 21, 2026",
    headline: "OpenCode sessions, attention alerts, and memory",
    bullets: [
      "macOS attention alerts use a glass toast that stays put while unrelated UI renders.",
      "The usage dot is back on the chat and headlines the tightest window, with usage refresh isolated per account.",
      "Mission Control can close a chat and hide worker sessions. The live badge counts active pinned workers.",
      "Memory extraction is proposed on the chat's own model, and safe memories can promote automatically.",
      "OpenCode turns stay on the session tree, the event stream reconnects with backoff, and subagent rows are labeled. A stall watchdog treats provider heartbeats as liveness rather than progress.",
      "GitHub offers Connect when no repository resolves, and a pasted GitHub link asks where it should open.",
    ],
  },
  {
    version: "0.5.9",
    date: "September 16, 2026",
    headline: "Database compatibility release",
    bullets: [
      "Includes the schema 58 migration for databases created by the current Bridge build.",
      "Ships as a new macOS version so the compatible DMG is selected instead of the older 0.5.8 build.",
    ],
  },
  {
    version: "0.5.8",
    date: "September 12, 2026",
    headline: "Worker reports reach the parent",
    bullets: [
      "Corrects updater public-key encoding; the 0.5.7 build was rejected before publication, and the signing key is unchanged.",
      "Worker results are saved with a pending notification and delivered in full when the parent can accept its next turn.",
      "Busy or disconnected parents retain pending reports, and peek can recover a completed worker's summary, tests, and remaining work.",
      "Request throttling no longer means an exhausted subscription, and rejected API keys get key-specific guidance.",
      "Provider switches no longer relabel old errors, and live and stored copies of one failure render only once.",
      "Changing credentials in an external terminal can still require refreshing the running provider process.",
    ],
  },
  {
    version: "0.5.6",
    date: "September 11, 2026",
    headline: "Images, agent setup, and Linux builds",
    bullets: [
      "Paste or upload an image straight into a Codex, OpenCode, or image-capable Cursor session.",
      "Setup can install an agent and run its provider sign-in, and a recovered login keeps the draft you were writing.",
      "Work shows only connected integration activity from the past 24 hours, timed by the source rather than by a cache refresh.",
      "Debian, AppImage, and Arch candidates build alongside the signed macOS disk image.",
    ],
  },
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
