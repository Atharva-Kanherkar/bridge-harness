// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { DISMISSED_WARNINGS_KEY, HealthWarnings, readDismissedWarnings } from "./HealthWarnings";
import type { HealthWarning } from "../types";

const tccWarning: HealthWarning = {
  id: "macos-tcc-protected-path",
  title: "Project folders sit inside macOS-protected locations",
  detail: "Repeated permission prompts — see “macOS file access prompts” in README.md.",
  paths: ["/Users/dev/Documents/app", "/Users/dev/Desktop/demo"],
};
const signatureWarning: HealthWarning = {
  id: "macos-adhoc-signature",
  title: "This build is ad-hoc signed — file-access grants reset on every rebuild",
  detail: "Sign development builds with a stable identity.",
  paths: [],
};

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => root.unmount()) };
}

function dismissButtons(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>('button[aria-label="Dismiss warning"]')];
}

// The component reads persisted dismissals during render, and jsdom here has
// no localStorage, so each case starts from a fresh storage stub.
beforeEach(() => {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    },
  });
});

describe("HealthWarnings", () => {

  it("renders nothing when the environment is clean", () => {
    expect(renderToStaticMarkup(<HealthWarnings warnings={[]} />)).toBe("");
  });

  it("renders each warning's title, guidance, and offending paths", () => {
    const html = renderToStaticMarkup(<HealthWarnings warnings={[tccWarning, signatureWarning]} />);
    expect(html).toContain("Project folders sit inside macOS-protected locations");
    expect(html).toContain("macOS file access prompts");
    expect(html).toContain("/Users/dev/Documents/app");
    expect(html).toContain("/Users/dev/Desktop/demo");
    expect(html).toContain("file-access grants reset on every rebuild");
    expect(html).toContain("Sign development builds with a stable identity.");
    // The paths list only exists on the warning that has paths.
    expect(html.match(/<ul/g)).toHaveLength(1);
  });

  it("closes only the warning that was dismissed", async () => {
    const { container, unmount } = await mount(<HealthWarnings warnings={[tccWarning, signatureWarning]} />);
    const closeSignature = dismissButtons(container)[1]!;
    await act(async () => closeSignature.click());
    expect(container.textContent).not.toContain("file-access grants reset on every rebuild");
    expect(container.textContent).toContain("Project folders sit inside macOS-protected locations");
    await unmount();
  });

  it("keeps a dismissed warning hidden after a remount, since detection keeps reporting it", async () => {
    const first = await mount(<HealthWarnings warnings={[signatureWarning]} />);
    await act(async () => dismissButtons(first.container)[0]!.click());
    await first.unmount();

    const second = await mount(<HealthWarnings warnings={[signatureWarning]} />);
    expect(second.container.textContent).toBe("");
    expect(readDismissedWarnings().has("macos-adhoc-signature")).toBe(true);
    await second.unmount();
  });

  it("renders nothing once every reported warning has been dismissed", async () => {
    localStorage.setItem(DISMISSED_WARNINGS_KEY, JSON.stringify(["macos-tcc-protected-path", "macos-adhoc-signature"]));
    const { container, unmount } = await mount(<HealthWarnings warnings={[tccWarning, signatureWarning]} />);
    expect(container.textContent).toBe("");
    await unmount();
  });

  it("ignores stored junk rather than hiding warnings the user never dismissed", async () => {
    localStorage.setItem(DISMISSED_WARNINGS_KEY, "{not json");
    const { container, unmount } = await mount(<HealthWarnings warnings={[signatureWarning]} />);
    expect(container.textContent).toContain("file-access grants reset on every rebuild");
    await unmount();
  });
});
