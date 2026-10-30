// Extra Claude Code and Codex sign-ins, read from each harness's
// `advanced.accounts`. Adapted from T3 Code's provider instances
// (https://github.com/pingdotgg/t3code, MIT; see THIRD_PARTY_NOTICES.md).
// The backend (`provider_accounts.rs`) is the authority on what is valid;
// this module only reads, labels, and builds new entries.

import type { Harness, HarnessConfig } from "./types";

export type ProviderAccount = {
  id: string;
  label: string;
  /** Claude: the CLAUDE_CONFIG_DIR. Codex: the shared CODEX_HOME (empty inherits). */
  homePath?: string;
  /** Codex only: the account's own home, linked to the shared one. */
  shadowHomePath?: string;
};

/** One account as the model picker offers it. */
export type AccountChoice = { harness: Harness; id: string; label: string };

export const ACCOUNT_HARNESSES = ["claude", "codex"] as const;

/** What differs between the two providers, kept here so components read a
 *  table rather than branching on which agent they show. */
export const ACCOUNT_PROVIDERS = {
  claude: {
    variable: "CLAUDE_CONFIG_DIR",
    ownLogin: "~/.claude, or CLAUDE_CONFIG_DIR when set",
    // Claude has no shared-home arrangement: each folder keeps its own chats.
    sharedHome: false,
    loginNote: null,
  },
  codex: {
    variable: "CODEX_HOME",
    ownLogin: "~/.codex, or CODEX_HOME when set",
    sharedHome: true,
    loginNote: "The account needs its own auth.json. If Codex keeps logins in the system keychain, switch it to file storage first.",
  },
} as const;

export function supportsAccounts(harness: string): harness is "claude" | "codex" {
  return (ACCOUNT_HARNESSES as readonly string[]).includes(harness);
}

/** The accounts stored on one harness. Malformed entries are skipped: the
 *  backend refuses to save them, so they only come from a hand-edited record. */
export function accountsOf(config: Pick<HarnessConfig, "advanced"> | undefined): ProviderAccount[] {
  const advanced = config?.advanced;
  if (!advanced || typeof advanced !== "object" || Array.isArray(advanced)) return [];
  const raw = (advanced as { accounts?: unknown }).accounts;
  if (!Array.isArray(raw)) return [];
  return raw.flatMap(entry => {
    if (!entry || typeof entry !== "object") return [];
    const { id, label, homePath, shadowHomePath } = entry as Record<string, unknown>;
    if (typeof id !== "string" || typeof label !== "string") return [];
    return [{
      id,
      label,
      ...(typeof homePath === "string" && homePath ? { homePath } : {}),
      ...(typeof shadowHomePath === "string" && shadowHomePath ? { shadowHomePath } : {}),
    }];
  });
}

/** Every enabled account-capable harness's accounts, in picker order. */
export function accountChoices(harnesses: readonly HarnessConfig[]): AccountChoice[] {
  return harnesses
    .filter(config => config.enabled && supportsAccounts(config.id))
    .flatMap(config => accountsOf(config).map(account => ({ harness: config.id as Harness, id: account.id, label: account.label })));
}

/** The account a chat runs on, normalised: the harness id and blank are the
 *  user's own login, the same as no account. */
export function normalizeAccount(harness: string, account: string | null | undefined): string | null {
  const trimmed = account?.trim();
  return trimmed && trimmed !== harness ? trimmed : null;
}

export function accountLabel(accounts: readonly AccountChoice[], harness: string, account: string | null | undefined): string | null {
  const id = normalizeAccount(harness, account);
  if (!id) return null;
  return accounts.find(choice => choice.harness === harness && choice.id === id)?.label ?? id;
}

/** A stable id from the name the user typed (T3's instance slug rules:
 *  a letter first, then letters, digits, - and _). */
export function accountIdFromLabel(label: string, harness: string, taken: readonly string[]): string {
  const base = label.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^[^a-z]+/, "").replace(/-+$/, "").slice(0, 48) || "account";
  const reserved = new Set([...taken, harness]);
  if (!reserved.has(base)) return base;
  for (let n = 2; ; n += 1) {
    const candidate = `${base}-${n}`;
    if (!reserved.has(candidate)) return candidate;
  }
}

/** A folder suggestion for a new account, e.g. `~/.claude_work`. */
export function suggestedFolder(harness: "claude" | "codex", id: string): string {
  return `~/.${harness}_${id.replace(/-/g, "_")}`;
}

/** The command that signs the account in, run once in a terminal. Uses the
 *  provider's own variable, never HOME, so the Keychain login stays put. */
export function loginCommand(harness: "claude" | "codex", account: Pick<ProviderAccount, "homePath" | "shadowHomePath">): string {
  if (harness === "claude") return `mkdir -p ${account.homePath} && CLAUDE_CONFIG_DIR=${account.homePath} claude auth login`;
  const home = account.shadowHomePath || account.homePath || "~/.codex";
  return `mkdir -p ${home} && CODEX_HOME=${home} codex login`;
}

/** Folder rules the backend enforces, checked before saving. */
export function folderError(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return "Choose a folder for this account.";
  if (!(trimmed === "~" || trimmed.startsWith("~/") || trimmed.startsWith("/"))) return "Use a folder that starts with / or ~/.";
  return null;
}
