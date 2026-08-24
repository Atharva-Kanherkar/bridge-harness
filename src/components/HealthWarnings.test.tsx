import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { HealthWarnings } from "./HealthWarnings";
import type { HealthWarning } from "../types";

describe("HealthWarnings", () => {
  it("renders nothing when the environment is clean", () => {
    expect(renderToStaticMarkup(<HealthWarnings warnings={[]} />)).toBe("");
  });

  it("renders each warning's title, guidance, and offending paths", () => {
    const warnings: HealthWarning[] = [
      {
        id: "macos-tcc-protected-path",
        title: "Project folders sit inside macOS-protected locations",
        detail: "Repeated permission prompts — see “macOS file access prompts” in README.md.",
        paths: ["/Users/dev/Documents/app", "/Users/dev/Desktop/demo"],
      },
      {
        id: "macos-adhoc-signature",
        title: "This build is ad-hoc signed — file-access grants reset on every rebuild",
        detail: "Sign development builds with a stable identity.",
        paths: [],
      },
    ];
    const html = renderToStaticMarkup(<HealthWarnings warnings={warnings} />);
    expect(html).toContain("Project folders sit inside macOS-protected locations");
    expect(html).toContain("macOS file access prompts");
    expect(html).toContain("/Users/dev/Documents/app");
    expect(html).toContain("/Users/dev/Desktop/demo");
    expect(html).toContain("file-access grants reset on every rebuild");
    expect(html).toContain("Sign development builds with a stable identity.");
    // The paths list only exists on the warning that has paths.
    expect(html.match(/<ul/g)).toHaveLength(1);
  });
});
