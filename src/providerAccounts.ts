// Extra sign-ins per coding agent (Claude Code, Codex, OpenCode, Cursor),
// read from each harness's `advanced.accounts`. Adapted from T3 Code's provider instances
// (https://github.com/pingdotgg/t3code, MIT; see THIRD_PARTY_NOTICES.md).
// The backend (`provider_accounts.rs`) is the authority on what is valid;
// this module only reads, labels, and builds new entries.

import type { Harness, HarnessConfig } from "./types";

export type ProviderAccount = {
  id: string;
  label: string;
  /** The folder the agent keeps this account's login in: CLAUDE_CONFIG_DIR,
   *  the shared CODEX_HOME (empty inherits), or OpenCode's data folder.
   *  Absent for agents whose accounts sign in with an API key. */
  homePath?: string;
  /** Codex only: the account's own home, linked to the shared one. */
  shadowHomePath?: string;
};

/** One account as the model picker offers it. */
export type AccountChoice = { harness: Harness; id: string; label: string };

/** How an account signs in: the agent's own CLI against the account's
 *  folder, a key Bridge hands to a private agent server for that folder, or
 *  an API key Bridge keeps in the macOS Keychain. */
export type AccountSignIn = "cli" | "folderKey" | "apiKey";

type AccountProvider = {
  signIn: AccountSignIn;
  /** The variable the account's folder (or key) reaches the agent through. */
  variable: string;
  /** Where the user's own login lives, for the "Your own login" row. */
  ownLogin: string;
  /** Codex only: accounts may share the main home's chats. */
  sharedHome: boolean;
  loginNote: string | null;
  /** The key an account signs in with, when it takes one in app. */
  key: { label: string; help: string; required: boolean; opencodeProvider?: string } | null;
};

/** What differs between agents, kept here so components read a table rather
 *  than branching on which agent they show. Mirrors `DRIVERS` in
 *  `provider_accounts.rs`; adding an agent is a row in each. */
export const ACCOUNT_PROVIDERS = {
  claude: {
    signIn: "cli",
    variable: "CLAUDE_CONFIG_DIR",
    ownLogin: "~/.claude, or CLAUDE_CONFIG_DIR when set",
    // Claude has no shared-home arrangement: each folder keeps its own chats.
    sharedHome: false,
    loginNote: null,
    key: null,
  },
  codex: {
    signIn: "cli",
    variable: "CODEX_HOME",
    ownLogin: "~/.codex, or CODEX_HOME when set",
    sharedHome: true,
    loginNote: "The account needs its own auth.json. If Codex keeps logins in the system keychain, switch it to file storage first.",
    key: null,
  },
  opencode: {
    signIn: "folderKey",
    variable: "XDG_DATA_HOME",
    ownLogin: "~/.local/share/opencode, or XDG_DATA_HOME when set",
    sharedHome: false,
    loginNote: "Only OpenCode's own data is kept apart. Everything else in the folder links to your usual data home, so tools the agent runs still find their installs.",
    key: {
      label: "OpenCode Go API key",
      help: "From opencode.ai. OpenCode stores it in this account's folder; Bridge does not keep it.",
      required: false,
      opencodeProvider: "opencode-go",
    },
  },
  cursor: {
    signIn: "apiKey",
    variable: "CURSOR_API_KEY",
    ownLogin: "cursor-agent login, kept in the Keychain",
    sharedHome: false,
    loginNote: "Cursor keeps its own login in a single Keychain item, so each extra account signs in with an API key from the Cursor dashboard instead.",
    key: {
      label: "Cursor API key",
      help: "Kept in the macOS Keychain, never in Bridge's settings.",
      required: true,
    },
  },
} as const satisfies Record<string, AccountProvider>;

export type AccountHarness = keyof typeof ACCOUNT_PROVIDERS;

export const ACCOUNT_HARNESSES = Object.keys(ACCOUNT_PROVIDERS) as AccountHarness[];

export function supportsAccounts(harness: string): harness is AccountHarness {
  return Object.prototype.hasOwnProperty.call(ACCOUNT_PROVIDERS, harness);
}

export function accountProvider(harness: AccountHarness): AccountProvider {
  return ACCOUNT_PROVIDERS[harness];
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
export function suggestedFolder(harness: AccountHarness, id: string): string {
  return `~/.${harness}_${id.replace(/-/g, "_")}`;
}

/** The command that signs the account in, run once in a terminal. Uses the
 *  provider's own variable, never HOME, so the Keychain login stays put. */
export function loginCommand(harness: AccountHarness, account: Pick<ProviderAccount, "homePath" | "shadowHomePath">): string | null {
  // Only agents that sign in with their own CLI have a command; the others
  // sign in inside Bridge.
  if (ACCOUNT_PROVIDERS[harness].signIn !== "cli") return null;
  if (harness === "claude") {
    const home = shellPath(account.homePath ?? "");
    return `mkdir -p ${home} && CLAUDE_CONFIG_DIR=${home} claude auth login`;
  }
  const home = shellPath(account.shadowHomePath || account.homePath || "~/.codex");
  return `mkdir -p ${home} && CODEX_HOME=${home} codex login`;
}

const SHELL_SAFE = /^[A-Za-z0-9_@%+=:,./-]+$/;

/** A folder as one shell word. A leading `~/` stays outside the quotes so the
 *  shell still expands it, in arguments and in `VAR=~/…` alike. */
export function shellPath(path: string): string {
  if (path === "~") return path;
  const home = path.startsWith("~/") ? "~/" : "";
  const rest = path.slice(home.length);
  if (!rest) return path;
  return home + (SHELL_SAFE.test(rest) ? rest : `'${rest.replace(/'/g, "'\\''")}'`);
}

/** Folder rules the backend enforces, checked before saving. */
export function folderError(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return "Choose a folder for this account.";
  if (!(trimmed === "~" || trimmed.startsWith("~/") || trimmed.startsWith("/"))) return "Use a folder that starts with / or ~/.";
  return null;
}
