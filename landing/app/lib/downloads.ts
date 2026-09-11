import { latestReleaseUrl, repoUrl } from "../content/site";

export type DownloadPlatform = "macos" | "linux";

const assetNames: Record<DownloadPlatform, RegExp> = {
  macos: /^Bridge_[\d.]+_(aarch64|arm64)\.dmg$/,
  linux: /^Bridge_[\d.]+_(amd64|x86_64)\.AppImage$/,
};

export function stableAssetUrl(release: unknown, platform: DownloadPlatform): string {
  if (!release || typeof release !== "object") return latestReleaseUrl;
  const data = release as Record<string, unknown>;
  if (data.draft !== false || data.prerelease !== false || !Array.isArray(data.assets)) {
    return latestReleaseUrl;
  }
  for (const asset of data.assets) {
    if (
      asset && typeof asset.name === "string" && assetNames[platform].test(asset.name) &&
      asset.state === "uploaded" && typeof asset.browser_download_url === "string" &&
      asset.browser_download_url.startsWith(`${repoUrl}/releases/download/`)
    ) {
      return asset.browser_download_url;
    }
  }
  return latestReleaseUrl;
}

export async function latestDownloadUrl(
  platform: DownloadPlatform,
  fetchRelease: typeof fetch = fetch,
): Promise<string> {
  try {
    const response = await fetchRelease(
      "https://api.github.com/repos/Atharva-Kanherkar/bridge-harness/releases/latest",
      {
        headers: { Accept: "application/vnd.github+json" },
        next: { revalidate: 300 },
        signal: AbortSignal.timeout(5000),
      },
    );
    if (response.ok) return stableAssetUrl(await response.json(), platform);
  } catch {
    // GitHub's release page remains useful during API outages or rate limiting.
  }
  return latestReleaseUrl;
}
