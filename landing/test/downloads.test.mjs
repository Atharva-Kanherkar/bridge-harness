import assert from "node:assert/strict";
import { test } from "node:test";
import { latestDownloadUrl, stableAssetUrl } from "../app/lib/downloads.ts";
import { latestReleaseUrl, repoUrl } from "../app/content/site.ts";

function asset(name) {
  return { name, state: "uploaded", browser_download_url: `${repoUrl}/releases/download/v0.5.6/${name}` };
}

const mac = asset("Bridge_0.5.6_aarch64.dmg");
const linux = asset("Bridge_0.5.6_amd64.AppImage");
const release = { draft: false, prerelease: false, assets: [mac, linux] };

test("selects each platform's published installer", () => {
  assert.equal(stableAssetUrl(release, "macos"), mac.browser_download_url);
  assert.equal(stableAssetUrl(release, "linux"), linux.browser_download_url);
});

test("ignores checksums, other architectures, and unfinished uploads", () => {
  const otherAssets = [
    asset("Bridge_0.5.6_aarch64.AppImage"),
    asset("Bridge_0.5.6_amd64.AppImage.sha256"),
    asset("Bridge_0.5.6_x64.dmg"),
    { ...linux, state: "new" },
  ];
  for (const platform of ["macos", "linux"]) {
    assert.equal(stableAssetUrl({ ...release, assets: otherAssets }, platform), latestReleaseUrl);
  }
});

test("a macOS-only release falls back for Linux", () => {
  assert.equal(stableAssetUrl({ ...release, assets: [mac] }, "linux"), latestReleaseUrl);
});

test("drafts, prereleases, and malformed data never become downloads", () => {
  for (const data of [null, {}, { ...release, draft: true }, { ...release, prerelease: true }, { ...release, assets: [null, {}] }]) {
    assert.equal(stableAssetUrl(data, "macos"), latestReleaseUrl);
  }
});

test("does not redirect to assets outside this repository", () => {
  const badAsset = { ...mac, browser_download_url: "https://example.com/Bridge.dmg" };
  assert.equal(stableAssetUrl({ ...release, assets: [badAsset] }, "macos"), latestReleaseUrl);
});

test("uses the stable API endpoint and a bounded cache and timeout", async () => {
  const result = await latestDownloadUrl("linux", async (url, options) => {
    assert.equal(url, "https://api.github.com/repos/Atharva-Kanherkar/bridge-harness/releases/latest");
    assert.equal(options.next.revalidate, 300);
    assert.ok(options.signal instanceof AbortSignal);
    return Response.json(release);
  });
  assert.equal(result, linux.browser_download_url);
});

test("API failures and invalid JSON fall back to the release page", async () => {
  for (const fetchRelease of [
    async () => new Response(null, { status: 403 }),
    async () => { throw new Error("network unavailable"); },
    async () => new Response("not JSON"),
  ]) {
    assert.equal(await latestDownloadUrl("macos", fetchRelease), latestReleaseUrl);
  }
});
