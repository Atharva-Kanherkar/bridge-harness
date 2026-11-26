import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ScreenHeading } from "./screen";

// The heading sits in the window minus the sidebar, and zoom narrows it
// further. These are the classes that decide whether a crowded header wraps
// its actions under the title or squeezes the title away beside them.

function parts(markup: string) {
  const title = /<div class="([^"]*)"><h1/.exec(markup)?.[1].split(" ") ?? [];
  const action = /<\/div><div class="([^"]*)">/.exec(markup)?.[1].split(" ") ?? [];
  return { title, action };
}

describe("ScreenHeading", () => {
  it("gives the title a real basis so the actions wrap instead of crushing it", () => {
    const { title } = parts(renderToStaticMarkup(<ScreenHeading title="Usage" action={<button>Refresh</button>} />));
    expect(title).toContain("grow");
    expect(title).toContain("basis-40");
    // `flex-1` is a zero basis: the row never wraps and the title shrinks to nothing.
    expect(title).not.toContain("flex-1");
  });

  it("keeps a described title wider before the actions drop under it", () => {
    const { title } = parts(renderToStaticMarkup(<ScreenHeading title="Reviews" description="What needs you." action={<button>Refresh</button>} />));
    expect(title).toContain("basis-80");
  });

  it("lets a long action cluster wrap inside the page instead of widening it", () => {
    const { action } = parts(renderToStaticMarkup(<ScreenHeading title="Reviews" action={<button>Refresh</button>} />));
    expect(action).toContain("flex-wrap");
    expect(action).toContain("max-w-full");
    expect(action).not.toContain("shrink-0");
  });
});
