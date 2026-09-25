const CODEX_RELEASE_URL = "https://api.github.com/repos/openai/codex/releases/latest";

type Version = { parts: [number, number, number]; prerelease: boolean };

function parseVersion(value: string): Version | undefined {
  const match = value.match(/(?:^|\D)v?(\d+)\.(\d+)\.(\d+)(-[\w.-]+)?/);
  if (!match) return undefined;
  return {
    parts: [Number(match[1]), Number(match[2]), Number(match[3])],
    prerelease: Boolean(match[4]),
  };
}

export function isOlderCodexVersion(installed: string, latest: string): boolean {
  const current = parseVersion(installed);
  const available = parseVersion(latest);
  if (!current || !available) return false;
  for (let index = 0; index < 3; index++) {
    if (current.parts[index] !== available.parts[index]) return current.parts[index] < available.parts[index];
  }
  return current.prerelease && !available.prerelease;
}

export function isCodexVersionError(message: string): boolean {
  return /Codex .+ is incompatible with Bridge\. Upgrade to Codex /i.test(message);
}

/** The CLI release feed is optional; offline Bridge stays usable. */
export async function latestCodexVersion(signal?: AbortSignal): Promise<string | undefined> {
  try {
    const response = await fetch(CODEX_RELEASE_URL, {
      headers: { Accept: "application/vnd.github+json" },
      signal,
    });
    if (!response.ok) return undefined;
    const release: unknown = await response.json();
    if (!release || typeof release !== "object" || !("tag_name" in release)) return undefined;
    const tag = release.tag_name;
    return typeof tag === "string" && parseVersion(tag) ? tag.replace(/^rust-v/, "") : undefined;
  } catch {
    return undefined;
  }
}
