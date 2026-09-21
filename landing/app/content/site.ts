export const repoUrl = "https://github.com/Atharva-Kanherkar/bridge-harness";
export const releasesUrl = `${repoUrl}/releases`;
export const latestReleaseUrl = `${repoUrl}/releases/latest`;
export const issuesUrl = `${repoUrl}/issues`;
export const docsPath = "/docs";
export const blogPath = "/blog";
export const downloadPath = "/download";
// Keep the primary CTA on the actual stable DMG. GitHub resolves `latest` to
// the newest non-draft release without making the landing page wait on the API.
export const macDownloadPath = `${repoUrl}/releases/latest/download/Bridge_0.5.9_aarch64.dmg`;
export const linuxDownloadPath = `${downloadPath}/linux`;
export const changelogPath = "/changelog";
export const comparePath = "/compare";
export const nodeRequirement = "Claude models need Node 18 or newer on your PATH.";
export const signingIdentity = "Developer ID Application: Yashaswi Kumar (3VN4X827YF)";

export function releaseNotesUrl(version: string) {
  return `${repoUrl}/releases/tag/v${version}`;
}
