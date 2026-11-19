// The arithmetic of a tool choice in the picker. It reads the fields that the
// choice of a harness holds, so a skill switch never asks which harness it is.
import { normalizeAccount } from "./providerAccounts";
import type { ChatToolItem, ChatToolSelection } from "./types";

/** A saved choice with each field set, for one harness and account. */
export type Choice = {
  harness: string;
  account: string | null;
  serversOff: string[];
  codexAppsOff: boolean;
  claudeSkills: { only: string[] } | null;
  codexSkillsOff: string[];
};

type ChoiceField = Exclude<keyof Choice, "harness" | "account">;

/**
 * The fields that the host check accepts in the choice of each harness. A
 * choice keeps its skills in at most one field: an allowlist or an off list.
 */
const CHOICE_FIELDS: Record<string, readonly ChoiceField[]> = {
  claude: ["serversOff", "claudeSkills"],
  codex: ["serversOff", "codexAppsOff", "codexSkillsOff"],
  opencode: ["serversOff"],
};

const holds = (harness: string, field: ChoiceField) => (CHOICE_FIELDS[harness] ?? []).includes(field);

/** Whether a saved choice belongs to this harness and this account. */
export function fits(choice: ChatToolSelection | null | undefined, harness: string, account: string | null): boolean {
  return !!choice && choice.harness === harness && normalizeAccount(choice.harness, choice.account) === account;
}

/** The saved choice as a full value for this harness and account. A choice for another harness or account starts over. */
export function baseChoice(choice: ChatToolSelection | null | undefined, harness: string, account: string | null): Choice {
  const own = fits(choice, harness, account) ? choice : null;
  return {
    harness,
    account,
    serversOff: [...own?.serversOff ?? []],
    codexAppsOff: own?.codexAppsOff ?? false,
    claudeSkills: own?.claudeSkills ? { only: [...own.claudeSkills.only] } : null,
    codexSkillsOff: [...own?.codexSkillsOff ?? []],
  };
}

const without = (list: string[], id: string) => list.filter(entry => entry !== id);
const marked = (list: string[], id: string, present: boolean) => present ? [...without(list, id), id] : without(list, id);

/** The choice after one switch. A first switch of an allowlist keeps each listed skill that is on. */
export function switched(base: Choice, item: ChatToolItem, on: boolean, items: ChatToolItem[]): Choice {
  if (item.kind === "server") return { ...base, serversOff: marked(base.serversOff, item.id, !on) };
  if (item.kind === "apps") return { ...base, codexAppsOff: !on };
  if (item.kind !== "skill") return base;
  if (holds(base.harness, "codexSkillsOff")) return { ...base, codexSkillsOff: marked(base.codexSkillsOff, item.id, !on) };
  if (!holds(base.harness, "claudeSkills")) return base;
  const only = base.claudeSkills?.only ?? items.filter(entry => entry.kind === "skill" && entry.state === "on").map(entry => entry.id);
  return { ...base, claudeSkills: { only: marked(only, item.id, on) } };
}

/**
 * The choice with each item on. It is the set of a chat with no choice, so a
 * chat that the harness did not resume with a changed set can return to it.
 */
export function allOn(base: Choice): Choice {
  return { ...base, serversOff: [], codexAppsOff: false, claudeSkills: null, codexSkillsOff: [] };
}

/** Whether a choice differs from each item on. A skill allowlist leaves out each skill that it does not name. */
export function leavesOut(choice: Choice): boolean {
  return choice.serversOff.length > 0 || choice.codexAppsOff || choice.claudeSkills !== null || choice.codexSkillsOff.length > 0;
}

/** The choice without a saved name that the sources no longer list. */
export function removed(base: Choice, item: ChatToolItem): Choice {
  if (item.kind === "server") return { ...base, serversOff: without(base.serversOff, item.id) };
  if (item.kind !== "skill") return base;
  return {
    ...base,
    claudeSkills: base.claudeSkills && { only: without(base.claudeSkills.only, item.id) },
    codexSkillsOff: without(base.codexSkillsOff, item.id),
  };
}

/** The listed items as a local choice sets them. Saved names the sources do not list stay visible. */
export function overlay(items: ChatToolItem[], choice: Choice): ChatToolItem[] {
  const off = new Set(choice.serversOff);
  const skillsOff = new Set(choice.codexSkillsOff);
  const only = choice.claudeSkills ? new Set(choice.claudeSkills.only) : null;
  const state = (on: boolean): ChatToolItem["state"] => on ? "on" : "off";
  const shown = items.map((item): ChatToolItem => {
    if (item.state === "fixed") return item;
    if (item.kind === "server") return { ...item, state: state(!off.has(item.id)) };
    if (item.kind === "apps") return { ...item, state: state(!choice.codexAppsOff) };
    if (item.kind === "skill") return { ...item, state: state(only ? only.has(item.id) : !skillsOff.has(item.id)) };
    return item;
  });
  const listed = (kind: ChatToolItem["kind"]) => new Set(items.filter(item => item.kind === kind).map(item => item.id));
  const servers = listed("server");
  const skills = listed("skill");
  const unseen = (kind: ChatToolItem["kind"], id: string, on: boolean): ChatToolItem =>
    ({ kind, id, label: id, group: kind === "server" ? "MCP servers" : "Skills", state: state(on), fixedReason: null, seen: false, tokens: null });
  shown.push(...choice.serversOff.filter(id => !servers.has(id)).map(id => unseen("server", id, false)));
  shown.push(...[...only ?? []].filter(id => !skills.has(id)).map(id => unseen("skill", id, true)));
  shown.push(...choice.codexSkillsOff.filter(id => !skills.has(id)).map(id => unseen("skill", id, false)));
  return shown;
}
