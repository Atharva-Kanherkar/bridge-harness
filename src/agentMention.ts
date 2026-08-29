import type { AgentDefinition } from "./types";

const AGENT_DIRECTIVE = /^#([A-Za-z0-9._-]+)(?:\s+([\s\S]*))?$/;

const ROLE_ALIASES: Record<string, readonly string[]> = {
  research: ["researcher", "research"],
  implementation: ["implementer", "implementation"],
  verification: ["verifier", "reviewer", "verification"],
  planning: ["planner", "planning"],
  documentation: ["documenter", "documentation", "docs"],
};

export type AgentMention = { token: string; objective: string };

export type AgentShortcutCandidate = {
  agent: AgentDefinition;
  token: string;
  searchTokens: string[];
};

/** Normalize configured labels and ids into the token vocabulary accepted after `#`. */
export function normalizeAgentToken(value: string): string {
  return value
    .trim()
    .replace(/^#/, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** A completed or empty leading `#agent` directive. Text containing `#` elsewhere is ordinary prose. */
export function parseAgentMention(text: string): AgentMention | null {
  const match = AGENT_DIRECTIVE.exec(text);
  if (!match) return null;
  return { token: normalizeAgentToken(match[1]), objective: (match[2] ?? "").trim() };
}

/** The leading `#token` currently being typed, before the objective begins. */
export function agentMentionQuery(text: string): string | undefined {
  return /^#([^\s]*)$/.exec(text)?.[1];
}

function aliasesFor(agent: AgentDefinition): string[] {
  return [...(ROLE_ALIASES[agent.role] ?? [])];
}

/**
 * Build one deterministic autocomplete row per enabled worker agent.
 * Role aliases are offered only when exactly one enabled agent owns that role;
 * otherwise selecting the row inserts its normalized configured name.
 */
export function agentShortcutCandidates(
  agents: AgentDefinition[],
  queryValue: string,
): AgentShortcutCandidate[] {
  const enabled = agents.filter(agent => agent.enabled && agent.role !== "orchestrator");
  const roleCounts = new Map<string, number>();
  for (const agent of enabled) roleCounts.set(agent.role, (roleCounts.get(agent.role) ?? 0) + 1);
  const query = normalizeAgentToken(queryValue);

  return enabled
    .map(agent => {
      const name = normalizeAgentToken(agent.name);
      const id = normalizeAgentToken(agent.id ?? "");
      const role = normalizeAgentToken(agent.role);
      const aliases = aliasesFor(agent);
      const token = roleCounts.get(agent.role) === 1 ? (aliases[0] ?? name ?? id) : (name || id);
      const searchTokens = [...new Set([token, name, id, role, ...aliases].filter(Boolean))];
      return { agent, token, searchTokens };
    })
    .filter(candidate => !query || candidate.searchTokens.some(token => token.includes(query)))
    .sort((left, right) => {
      const leftPrefix = Number(left.searchTokens.some(token => token.startsWith(query)));
      const rightPrefix = Number(right.searchTokens.some(token => token.startsWith(query)));
      if (leftPrefix !== rightPrefix) return rightPrefix - leftPrefix;
      const tokenOrder = left.token.localeCompare(right.token);
      return tokenOrder || (left.agent.id ?? "").localeCompare(right.agent.id ?? "");
    });
}
