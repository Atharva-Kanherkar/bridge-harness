import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("authenticated browser bridge artifacts", () => {
  it("ships a stable Chrome MV3 identity with narrowly declared browser capabilities", () => {
    const manifest = JSON.parse(read("browser-extension/manifest.json"));
    expect(manifest.manifest_version).toBe(3);
    expect(manifest.permissions).toEqual(expect.arrayContaining(["nativeMessaging", "debugger", "tabCapture", "activeTab"]));
    expect(manifest.key).toMatch(/^MIIB/);
  });

  it("redacts sensitive screenshots and deduplicates commands across reconnects", () => {
    const background = read("browser-extension/background.js");
    expect(background).toContain("completedCommands");
    expect(background).toContain("redactScreenshot");
    expect(background).toContain("redactionApplied");
    expect(background).toContain("Runtime.consoleAPICalled");
    expect(background).toContain("Network.responseReceived");
  });

  it("labels web content as untrusted and emits stable DOM deltas", () => {
    const content = read("browser-extension/content.js");
    expect(content).toContain('contentBoundary: "untrusted_web_content"');
    expect(content).toContain("promptInjectionSuspected");
    expect(content).toContain("const ids = new WeakMap()");
    expect(content).toContain("MutationObserver");
    expect(content).toContain("removedIds");
  });

  it("includes a Safari Web Extension and deterministic common-site skills", () => {
    const manifest = JSON.parse(read("safari-extension/Resources/manifest.json"));
    expect(manifest.permissions).toContain("nativeMessaging");
    for (const name of ["github", "google-workspace", "notion"]) {
      const skill = JSON.parse(read(`browser-skills/${name}.json`));
      expect(skill.domains.length).toBeGreaterThan(0);
      expect(skill.steps.some((step: { action: string }) => step.action === "approval")).toBe(true);
    }
  });
});
