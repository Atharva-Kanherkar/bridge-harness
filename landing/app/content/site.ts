export const repoUrl = "https://github.com/Atharva-Kanherkar/bridge-harness";
export const releasesUrl = `${repoUrl}/releases`;
export const latestReleaseUrl = `${repoUrl}/releases/latest`;
export const issuesUrl = `${repoUrl}/issues`;
export const docsUrl = `${repoUrl}/tree/main/docs`;
export const downloadPath = "/download";
export const changelogPath = "/changelog";
export const latestVersion = "0.5.5";
export const dmgName = `Bridge_${latestVersion}_aarch64.dmg`;
export const platformLabel = "macOS 12 or later · Apple Silicon";
export const nodeRequirement = "Claude models need Node 18 or newer on your PATH.";
export const signingIdentity = "Developer ID Application: Yashaswi Kumar (3VN4X827YF)";

export function releaseNotesUrl(version: string) {
  return `${repoUrl}/releases/tag/v${version}`;
}
