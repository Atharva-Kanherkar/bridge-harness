import { describe, expect, it } from "vitest";
import type { HarnessConfig } from "./types";
import {
  ACCOUNT_HARNESSES, ACCOUNT_PROVIDERS, accountChoices, accountIdFromLabel, accountLabel, accountsOf, folderError, loginCommand, normalizeAccount, shellPath, suggestedFolder, supportsAccounts,
} from "./providerAccounts";

const config = (id: string, advanced: unknown, enabled = true) =>
  ({ id, label: id, enabled, defaultModel: null, effort: null, systemPrompt: "", advanced, isOverride: true }) as HarnessConfig;

describe("provider accounts", () => {
  it("reads accounts and skips malformed entries", () => {
    expect(accountsOf(config("claude", { workerCredentialSource: "auto", accounts: [
      { id: "personal", label: "Personal", homePath: "~/.claude_personal" },
      { id: 3, label: "Broken" },
      null,
    ] }))).toEqual([{ id: "personal", label: "Personal", homePath: "~/.claude_personal" }]);
    expect(accountsOf(config("claude", {}))).toEqual([]);
    expect(accountsOf(config("claude", { accounts: "nope" }))).toEqual([]);
    expect(accountsOf(undefined)).toEqual([]);
  });

  it("offers accounts only for enabled agents that take them", () => {
    const accounts = [{ id: "work", label: "Work", homePath: "~/.x" }];
    expect(accountChoices([
      config("claude", { accounts }),
      config("codex", { accounts }, false),
      config("opencode", { accounts }),
      config("cursor", { accounts: [{ id: "team", label: "Team" }] }),
      config("grok", { accounts }),
    ])).toEqual([
      { harness: "claude", id: "work", label: "Work" },
      { harness: "opencode", id: "work", label: "Work" },
      { harness: "cursor", id: "team", label: "Team" },
    ]);
    // Mirrors DRIVERS in provider_accounts.rs.
    expect(ACCOUNT_HARNESSES).toEqual(["claude", "codex", "opencode", "cursor"]);
    expect(supportsAccounts("codex")).toBe(true);
    expect(supportsAccounts("grok")).toBe(false);
    expect(supportsAccounts("toString")).toBe(false);
  });

  it("signs OpenCode and Cursor accounts in inside Bridge, not with a CLI command", () => {
    expect(loginCommand("opencode", { homePath: "~/.opencode_go" })).toBeNull();
    expect(loginCommand("cursor", {})).toBeNull();
    expect(ACCOUNT_PROVIDERS.opencode.signIn).toBe("folderKey");
    expect(ACCOUNT_PROVIDERS.cursor.signIn).toBe("apiKey");
    expect(ACCOUNT_PROVIDERS.cursor.key.required).toBe(true);
    expect(suggestedFolder("opencode", "go")).toBe("~/.opencode_go");
  });

  it("treats the harness id and blank as the user's own login", () => {
    expect(normalizeAccount("claude", "claude")).toBeNull();
    expect(normalizeAccount("claude", " ")).toBeNull();
    expect(normalizeAccount("claude", null)).toBeNull();
    expect(normalizeAccount("claude", "work")).toBe("work");
    const choices = [{ harness: "claude" as const, id: "work", label: "Work" }];
    expect(accountLabel(choices, "claude", "work")).toBe("Work");
    expect(accountLabel(choices, "claude", "claude")).toBeNull();
    expect(accountLabel(choices, "claude", "gone")).toBe("gone");
  });

  it("derives ids the backend accepts", () => {
    expect(accountIdFromLabel("Personal", "claude", [])).toBe("personal");
    expect(accountIdFromLabel("My Work Plan!", "claude", [])).toBe("my-work-plan");
    expect(accountIdFromLabel("2nd team", "claude", [])).toBe("nd-team");
    expect(accountIdFromLabel("Claude", "claude", [])).toBe("claude-2");
    expect(accountIdFromLabel("Work", "codex", ["work", "work-2"])).toBe("work-3");
    expect(accountIdFromLabel("!!!", "codex", [])).toBe("account");
    expect(suggestedFolder("claude", "my-work")).toBe("~/.claude_my_work");
  });

  it("builds the sign-in command with the provider's own variable", () => {
    expect(loginCommand("claude", { homePath: "~/.claude_personal" })).toBe("mkdir -p ~/.claude_personal && CLAUDE_CONFIG_DIR=~/.claude_personal claude auth login");
    expect(loginCommand("codex", { shadowHomePath: "~/.codex_personal" })).toBe("mkdir -p ~/.codex_personal && CODEX_HOME=~/.codex_personal codex login");
    expect(loginCommand("codex", { homePath: "/srv/codex" })).toBe("mkdir -p /srv/codex && CODEX_HOME=/srv/codex codex login");
  });

  it("quotes folders the shell would split or interpret", () => {
    expect(loginCommand("codex", { homePath: "/tmp/Bridge Accounts/codex" }))
      .toBe("mkdir -p '/tmp/Bridge Accounts/codex' && CODEX_HOME='/tmp/Bridge Accounts/codex' codex login");
    expect(loginCommand("claude", { homePath: "~/Library/Application Support/Claude Work" }))
      .toBe("mkdir -p ~/'Library/Application Support/Claude Work' && CLAUDE_CONFIG_DIR=~/'Library/Application Support/Claude Work' claude auth login");
    expect(shellPath("~/a;rm -rf $HOME `x` & y")).toBe("~/'a;rm -rf $HOME `x` & y'");
    expect(shellPath("/tmp/it's")).toBe("'/tmp/it'\\''s'");
    expect(shellPath("~/.claude_work")).toBe("~/.claude_work");
    expect(shellPath("~")).toBe("~");
  });

  it("checks folders the way the backend does", () => {
    expect(folderError("")).toMatch(/Choose a folder/);
    expect(folderError("relative/dir")).toMatch(/starts with/);
    expect(folderError("~/.claude_x")).toBeNull();
    expect(folderError("/abs")).toBeNull();
  });
});
