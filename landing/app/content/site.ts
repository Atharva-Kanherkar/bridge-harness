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
// An ad-hoc signed macOS build, served by /download/macos only while it is
// newer than the latest notarized release. Set to null once one catches up.
export const macAdhocBuild: { version: string; url: string } | null = {
  version: "0.10.0",
  url: `${repoUrl}/releases/download/adhoc-2026-11-27/Bridge_0.10.0_aarch64_adhoc.dmg`,
};
export const signingIdentity = "Developer ID Application: Yashaswi Kumar (3VN4X827YF)";

export function releaseNotesUrl(version: string) {
  return `${repoUrl}/releases/tag/v${version}`;
}
