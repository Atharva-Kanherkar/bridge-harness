// What the app knows about an API key without its value: how to name its kind,
// which sites it usually belongs on, and what its placeholder looks like. The
// kinds mirror `secret_interception`'s detectors in bridge-core.

const KIND_LABELS: Record<string, string> = {
  openai: "OpenAI key",
  anthropic: "Anthropic key",
  github: "GitHub token",
  slack: "Slack token",
  notion: "Notion key",
  bearer: "Bearer token",
  jwt: "JWT",
  credential_assignment: "Credential",
};

export const keyKindLabel = (kind: string): string => KIND_LABELS[kind] ?? "API key";

/** Where a key of this kind usually goes; the sites a pasted one starts with. */
export const DEFAULT_KEY_SITES: Record<string, string[]> = {
  openai: ["api.openai.com"],
  anthropic: ["api.anthropic.com"],
  github: ["api.github.com", "uploads.github.com"],
  slack: ["slack.com"],
  notion: ["api.notion.com"],
};

/** The name a key of this kind is usually saved under. */
export const DEFAULT_KEY_NAMES: Record<string, string> = {
  openai: "OPENAI_API_KEY",
  anthropic: "ANTHROPIC_API_KEY",
  github: "GITHUB_TOKEN",
  slack: "SLACK_TOKEN",
  notion: "NOTION_API_KEY",
};

/** Recognize a pasted value by its prefix, the way the core's detectors do. */
export function guessKeyKind(value: string): string {
  const key = value.trim();
  if (/^sk-ant-/.test(key)) return "anthropic";
  if (/^sk-/.test(key)) return "openai";
  if (/^(github_pat_|gh[pousr]_)/.test(key)) return "github";
  if (/^xox[baprs]-/.test(key)) return "slack";
  if (/^(secret|ntn)_/.test(key)) return "notion";
  return "custom";
}

/** Names read like environment variables: OPENAI_API_KEY. */
export const isValidKeyName = (name: string): boolean => /^[A-Z][A-Z0-9_]{1,63}$/.test(name.trim());

/** Turn what someone typed into a name: upper case, underscores for the rest. */
export const toKeyName = (input: string): string => input.toUpperCase().replace(/[^A-Z0-9_]+/g, "_");

/** Split a free-text site list ("api.x.com, *.y.com") into entries. */
export const parseSites = (input: string): string[] =>
  input.split(/[\s,]+/).map(site => site.trim()).filter(Boolean);

/** A key placeholder in chat text: `[secret:sec_…]` or `[secret:NAME]`. */
export const SECRET_PLACEHOLDER = /(\[secret:[A-Za-z0-9_]{1,80}\])/g;
export const placeholderReference = (token: string): string | undefined =>
  /^\[secret:([A-Za-z0-9_]{1,80})\]$/.exec(token)?.[1];
