// The personal-app evals: what a person asks for, and what a good answer is.
//
// Prompts are phrased the way a person would ask, never naming Bridge tools,
// so whether the chat finds `apps_draft` is part of the test. Slack is never
// used. Real servers only ever get reads: a draft's writes are previewed
// (Bridge returns what would be sent) and are never approved here.

/** One app to build: the chat agent, the server it is for, and whether a
 *  good app needs a write. */
export const BUILD_CASES = [
  {
    id: "docs-codex",
    chat: "codex",
    server: "openaiDeveloperDocs",
    prompt: "Make me a personal app for the OpenAI developer docs MCP server: I want to search the docs and read a page right here in the chat.",
    needsWrites: false,
  },
  {
    id: "docs-claude",
    chat: "claude",
    server: "openaiDeveloperDocs",
    prompt: "Make me a personal app for the OpenAI developer docs MCP server: I want to search the docs and read a page right here in the chat.",
    needsWrites: false,
  },
  {
    id: "docs-opencode",
    chat: "opencode",
    server: "openaiDeveloperDocs",
    prompt: "Make me a personal app for the OpenAI developer docs MCP server: I want to search the docs and read a page right here in the chat.",
    needsWrites: false,
  },
  {
    id: "firestore-codex",
    chat: "codex",
    server: "firestore",
    prompt: "Make me a personal app for my firestore MCP server: browse my collections, open a document, and add a new document to a collection.",
    needsWrites: true,
  },
  {
    id: "memory-codex",
    chat: "codex",
    server: "agentic-memory",
    prompt: "Make me a personal app for my agentic-memory MCP server: search my memories, and save a new memory.",
    needsWrites: true,
  },
  {
    id: "railway-codex",
    chat: "codex",
    server: "railway",
    prompt: "Make me a personal app for my Railway MCP server: list my projects and services, show recent deployment logs for a service, and a button to redeploy it.",
    needsWrites: true,
  },
  {
    id: "claude-docs-claude",
    chat: "claude",
    server: "claude.ai Claude Docs",
    // The connector opens a doc by its id or link and cannot list or search
    // docs, so the app takes a link (asking it to "find" a doc made the chat
    // go looking for one instead of building).
    prompt: "Make me a personal app for my Claude Docs connector: I paste a doc link and read the doc right here in the chat.",
    needsWrites: false,
  },
];

/** Asks for the routing eval. `expect` is the build case whose locked app
 *  should open, or null when no app should. Every ask is read-only: a chat
 *  may answer with the server's own tools instead of opening an app, and
 *  nothing here may lead it to write. */
export const ROUTING_ASKS = [
  { id: "r-docs-1", ask: "Open my OpenAI docs search app.", expect: "docs-codex" },
  { id: "r-docs-2", ask: "I want to look something up in the OpenAI developer docs myself.", expect: "docs-codex" },
  { id: "r-docs-3", ask: "Give me the docs browser so I can find the Realtime API guide.", expect: "docs-codex" },
  { id: "r-fs-1", ask: "Let me browse my Firestore collections.", expect: "firestore-codex" },
  { id: "r-fs-2", ask: "I need to look at a document in Firestore.", expect: "firestore-codex" },
  { id: "r-mem-1", ask: "Open my memory search app.", expect: "memory-codex" },
  { id: "r-mem-2", ask: "Let me search through my saved memories.", expect: "memory-codex" },
  { id: "r-rw-1", ask: "Show me my Railway deployment logs.", expect: "railway-codex" },
  { id: "r-rw-2", ask: "Open the app for my Railway services.", expect: "railway-codex" },
  { id: "r-cd-1", ask: "Open my Claude Docs reader.", expect: "claude-docs-claude" },
  { id: "r-cd-2", ask: "I want to find and read one of my Claude docs.", expect: "claude-docs-claude" },
  { id: "r-docs-4", ask: "Pull up the app for searching OpenAI documentation, prefilled with 'structured outputs'.", expect: "docs-codex" },
  { id: "r-neg-1", ask: "What is 17 times 23?", expect: null },
  { id: "r-neg-2", ask: "Write a haiku about autumn.", expect: null },
  { id: "r-neg-3", ask: "Explain the difference between TCP and UDP in two sentences.", expect: null },
  { id: "r-neg-4", ask: "What's a personal app in Bridge, in one sentence? Don't open anything.", expect: null },
  { id: "r-neg-5", ask: "What is Firestore, the product, in one sentence?", expect: null },
  { id: "r-neg-6", ask: "What is Railway the company known for? One sentence.", expect: null },
  { id: "r-neg-7", ask: "Give me three tips for remembering names.", expect: null },
  { id: "r-neg-8", ask: "Summarize the plot of Hamlet in two sentences.", expect: null },
];
