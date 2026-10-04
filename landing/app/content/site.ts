export const repoUrl = "https://github.com/Atharva-Kanherkar/bridge-harness";
export const releasesUrl = `${repoUrl}/releases`;
export const latestReleaseUrl = `${repoUrl}/releases/latest`;
export const issuesUrl = `${repoUrl}/issues`;
export const docsPath = "/docs";
export const blogPath = "/blog";
export const downloadPath = "/download";
// The DMG name carries the version, so resolve it per request instead of
// hardcoding a release that goes stale on the next tag.
export const macDownloadPath = `${downloadPath}/macos`;
export const linuxDownloadPath = `${downloadPath}/linux`;
export const changelogPath = "/changelog";
export const comparePath = "/compare";
export const nodeRequirement = "Claude models need Node 18 or newer on your PATH.";
export const signingIdentity = "Developer ID Application: Yashaswi Kumar (3VN4X827YF)";

export function releaseNotesUrl(version: string) {
  return `${repoUrl}/releases/tag/v${version}`;
}
