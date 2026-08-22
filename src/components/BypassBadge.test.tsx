import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BypassBadge } from "./BypassBadge";

describe("BypassBadge", () => {
  it("stands in the chrome while approvals are bypassed", () => {
    const html = renderToStaticMarkup(<BypassBadge bypassing onOpenSettings={() => undefined} />);
    expect(html).toContain("Approvals bypassed");
    // The way out is on the badge itself, not only in settings.
    expect(html).toContain("<button");
  });

  it("renders nothing when Bridge is still asking", () => {
    expect(renderToStaticMarkup(<BypassBadge bypassing={false} onOpenSettings={() => undefined} />)).toBe("");
  });
});
