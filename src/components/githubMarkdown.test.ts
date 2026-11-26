import { describe, expect, it } from "vitest";
import { normalizeGithubMarkdown, resolveReferences, splitGithubDetails, splitGithubMedia } from "./githubMarkdown";

const REPO = "https://github.com/bridge/harness";

describe("normalizeGithubMarkdown", () => {
  it("drops template comments and turns <br> into a real break", () => {
    const body = normalizeGithubMarkdown("Intro\n<!-- please keep this section -->\nafter<br>next");
    expect(body).not.toContain("please keep");
    expect(body).toBe("Intro\nafter\nnext");
  });

  it("renders task lists as glyphs rather than literal brackets", () => {
    const body = normalizeGithubMarkdown("- [x] shipped\n- [ ] pending");
    expect(body).toBe("- ☑ shipped\n- ☐ pending");
  });

  it("labels images as links instead of fetching remote pixels", () => {
    expect(normalizeGithubMarkdown("![the pane](https://img.test/a.png)"))
      .toBe("[image: the pane](https://img.test/a.png)");
    expect(normalizeGithubMarkdown('<img width="600" src="https://img.test/b.png" alt="x">'))
      .toBe("[image: x](https://img.test/b.png)");
    expect(normalizeGithubMarkdown('<img width="600" src="https://img.test/b.png">'))
      .toBe("[image](https://img.test/b.png)");
  });

  it("only links schemes the shell will open, and keeps the rest as text", () => {
    // The pane renders inside a Tauri webview with no CSP, and the click
    // interceptor only claims http(s)/mailto — so any other scheme would keep
    // the webview's default action. None of these may become an href.
    for (const hostile of ["javascript:void%200", "data:text/html,<script>x</script>", "file:///etc/passwd", "vbscript:msgbox"]) {
      expect(normalizeGithubMarkdown(`<a href="${hostile}">click me</a>`)).toBe("click me");
      expect(normalizeGithubMarkdown(`![shot](${hostile})`)).toBe("image: shot");
      expect(normalizeGithubMarkdown(`<img src="${hostile}">`)).toBe("image");
    }
    // A hostile link the author wrote *as markdown* passes through here — it
    // is the renderer that refuses to make it an anchor, and that is the one
    // boundary, covered in Markdown.test.tsx.
    // The allowlisted schemes still link.
    expect(normalizeGithubMarkdown('<a href="https://example.test/x">docs</a>'))
      .toBe("[docs](https://example.test/x)");
    expect(normalizeGithubMarkdown('<a href="mailto:me@example.test">mail</a>'))
      .toBe("[mail](mailto:me@example.test)");
    expect(normalizeGithubMarkdown("![shot](https://img.test/a.png)"))
      .toBe("[image: shot](https://img.test/a.png)");
  });

  it("rewrites allowlisted formatting tags and leaves scripts verbatim", () => {
    expect(normalizeGithubMarkdown("<b>bold</b> and <code>x</code>")).toBe("**bold** and `x`");
    // The inertness boundary: unknown markup is text, never markup.
    expect(normalizeGithubMarkdown("<script>window.pwned = true</script>"))
      .toBe("<script>window.pwned = true</script>");
  });

  it("links bare URLs once and never relinks an existing link", () => {
    expect(normalizeGithubMarkdown("see https://example.test/x for more"))
      .toBe("see [https://example.test/x](https://example.test/x) for more");
    expect(normalizeGithubMarkdown("[docs](https://example.test/x)"))
      .toBe("[docs](https://example.test/x)");
  });

  it("links issue references and handles, and only when they are references", () => {
    const body = normalizeGithubMarkdown("Fixes #412 for @atharva.", REPO);
    expect(body).toBe(`Fixes [#412](${REPO}/issues/412) for [@atharva](https://github.com/atharva).`);
    // No repository context means no dead link.
    expect(normalizeGithubMarkdown("Fixes #412 for @atharva")).toBe("Fixes #412 for @atharva");
    // An email address is not a mention.
    expect(normalizeGithubMarkdown("mail me@example.com", REPO)).toContain("me@example.com");
    expect(normalizeGithubMarkdown("mail me@example.com", REPO)).not.toContain("github.com/example");
  });

  it("sends an Enterprise mention to the Enterprise host, not to github.com", () => {
    const enterprise = "https://github.acme-corp.test/platform/bridge";
    const body = normalizeGithubMarkdown("ping @atharva on #7", enterprise);
    expect(body).toBe(
      `ping [@atharva](https://github.acme-corp.test/atharva) on [#7](${enterprise}/issues/7)`,
    );
    expect(body).not.toContain("github.com");
  });

  it("leaves fenced code exactly as authored", () => {
    const source = "before #1\n```ts\nconst x = \"#1 @user https://example.test\";\n```\nafter #1";
    const body = normalizeGithubMarkdown(source, REPO);
    expect(body).toContain('const x = "#1 @user https://example.test";');
    expect(body).toContain(`before [#1](${REPO}/issues/1)`);
    expect(body).toContain(`after [#1](${REPO}/issues/1)`);
  });

  it("leaves inline code spans alone, including markup they document", () => {
    expect(normalizeGithubMarkdown("use `#1` not #1", REPO))
      .toBe(`use \`#1\` not [#1](${REPO}/issues/1)`);
    // A code span that *documents* markup keeps it verbatim; rewriting inside
    // one is the opposite of what a code span means.
    expect(normalizeGithubMarkdown("show `<b>x</b>` and `<br>` but not <b>y</b>"))
      .toBe("show `<b>x</b>` and `<br>` but not **y**");
    expect(normalizeGithubMarkdown("`<!-- kept -->` <!-- dropped -->"))
      .toBe("`<!-- kept -->`");
    expect(normalizeGithubMarkdown("`- [x] literal`")).toBe("`- [x] literal`");
    expect(normalizeGithubMarkdown("`@handle`", REPO)).toBe("`@handle`");
  });

  it("cannot be tricked into forging a code-span placeholder", () => {
    // The mask sentinels are private-use code points; any arriving in the
    // source are stripped before masking so a body cannot smuggle text past
    // the rewrites by pretending to be a restored span.
    // The sentinels are removed and their digits stay ordinary text; what
    // must not happen is the pair being read back as a restored code span.
    const body = normalizeGithubMarkdown("\uE0000\uE001 <b>y</b>");
    expect(body).toBe("0 **y**");
    expect(body).not.toContain("\uE000");
    // Real code spans in the same body still survive intact.
    expect(normalizeGithubMarkdown("\uE0000\uE001 `<b>z</b>`")).toBe("0 `<b>z</b>`");
  });
});

describe("splitGithubDetails", () => {
  it("splits a body into prose and collapsible sections", () => {
    const blocks = splitGithubDetails("Intro text\n<details><summary>CI log</summary>\n\nthe log\n\n</details>\nOutro");
    expect(blocks).toEqual([
      { kind: "markdown", text: "Intro text\n" },
      { kind: "details", summary: "CI log", body: "the log" },
      { kind: "markdown", text: "\nOutro" },
    ]);
  });

  it("names an unlabelled section and strips markup out of the summary", () => {
    const [block] = splitGithubDetails("<details><summary><b>Trace</b></summary>body</details>");
    expect(block).toEqual({ kind: "details", summary: "Trace", body: "body" });
    const [bare] = splitGithubDetails("<details>body</details>");
    expect(bare).toEqual({ kind: "details", summary: "Details", body: "body" });
  });

  it("degrades an unbalanced section to one markdown run", () => {
    expect(splitGithubDetails("<details><summary>oops</summary>no close"))
      .toEqual([{ kind: "markdown", text: "<details><summary>oops</summary>no close" }]);
  });

  it("always returns at least one run for an empty body", () => {
    expect(splitGithubDetails("")).toEqual([{ kind: "markdown", text: "" }]);
  });

  it("leaves a fenced <details> example as code instead of rendering it", () => {
    const source = "Write it like this:\n\n```html\n<details><summary>x</summary>y</details>\n```\n";
    expect(splitGithubDetails(source)).toEqual([{ kind: "markdown", text: source }]);
  });

  it("still finds a real section after a fenced example of one", () => {
    const source = "```\n<details>fenced</details>\n```\n<details><summary>Real</summary>body</details>";
    const blocks = splitGithubDetails(source);
    expect(blocks).toHaveLength(2);
    expect(blocks[0]).toEqual({ kind: "markdown", text: "```\n<details>fenced</details>\n```\n" });
    expect(blocks[1]).toEqual({ kind: "details", summary: "Real", body: "body" });
  });

  it("does not close a section on a fenced </details>", () => {
    const blocks = splitGithubDetails("<details><summary>Log</summary>\n\n```\n</details>\n```\n\n</details>");
    expect(blocks).toHaveLength(1);
    expect(blocks[0].kind).toBe("details");
    expect(blocks[0]).toMatchObject({ summary: "Log", body: "```\n</details>\n```" });
  });
});

describe("splitGithubMedia", () => {
  const media = (text: string) => splitGithubMedia(text).flatMap(block => block.kind === "media" ? block.items : []);

  it("lifts markdown and HTML images out of the text around them", () => {
    const blocks = splitGithubMedia('Before\n\n![a shot](https://github.com/a.png "title")\n\nAfter <img width="300" alt="inline" src="https://github.com/b.png"> the fact');
    expect(blocks).toEqual([
      { kind: "markdown", text: "Before\n" },
      { kind: "media", items: [{ kind: "image", url: "https://github.com/a.png", alt: "a shot" }] },
      { kind: "markdown", text: "\nAfter  the fact" },
      { kind: "media", items: [{ kind: "image", url: "https://github.com/b.png", alt: "inline" }] },
    ]);
  });

  it("puts consecutive media lines in one row and keeps a linked image's target", () => {
    const blocks = splitGithubMedia("![one](https://github.com/1.png)\n[![badge](https://github.com/2.svg)](https://example.test/ci)\n<a href=\"https://example.test/big\"><img src=\"https://github.com/3.png\"></a>");
    expect(blocks).toEqual([{ kind: "media", items: [
      { kind: "image", url: "https://github.com/1.png", alt: "one" },
      { kind: "image", url: "https://github.com/2.svg", alt: "badge", href: "https://example.test/ci" },
      { kind: "image", url: "https://github.com/3.png", alt: "", href: "https://example.test/big" },
    ] }]);
  });

  it("treats an uploaded recording, a video file and a <video> tag as videos", () => {
    expect(media([
      "https://github.com/user-attachments/assets/0f0f0f0f-aaaa-bbbb-cccc-121212121212",
      "",
      "https://github.com/bridge/harness/assets/42/0f0f0f0f-aaaa-bbbb-cccc-121212121212",
      "",
      "![demo](https://raw.githubusercontent.com/o/r/main/demo.mp4)",
      "",
      '<video src="https://example.test/clip.webm" controls></video>',
      "",
      "<video controls>\n  <source src=\"https://example.test/wrapped.mov\">\n</video>".replace(/\n/g, " "),
    ].join("\n")).map(item => [item.kind, item.url])).toEqual([
      ["video", "https://github.com/user-attachments/assets/0f0f0f0f-aaaa-bbbb-cccc-121212121212"],
      ["video", "https://github.com/bridge/harness/assets/42/0f0f0f0f-aaaa-bbbb-cccc-121212121212"],
      ["video", "https://raw.githubusercontent.com/o/r/main/demo.mp4"],
      ["video", "https://example.test/clip.webm"],
      ["video", "https://example.test/wrapped.mov"],
    ]);
  });

  it("leaves an ordinary address on its own line as a link, not a player", () => {
    expect(media("https://github.com/bridge/harness/pull/12\n\nhttps://example.test/page")).toEqual([]);
  });

  it("turns a screenshot table into captioned rows and leaves a prose table alone", () => {
    const table = "| Dark | **Light** |\n|---|---|\n| ![d](https://github.com/d.png) | ![l](https://github.com/l.png) |\n| ![d2](https://github.com/d2.png) | |";
    expect(splitGithubMedia(table)).toEqual([
      { kind: "media", items: [
        { kind: "image", url: "https://github.com/d.png", alt: "d", caption: "Dark" },
        { kind: "image", url: "https://github.com/l.png", alt: "l", caption: "Light" },
      ] },
      { kind: "media", items: [{ kind: "image", url: "https://github.com/d2.png", alt: "d2", caption: "Dark" }] },
    ]);
    const prose = "| Step | Result |\n|---|---|\n| build | ok |";
    expect(splitGithubMedia(prose)).toEqual([{ kind: "markdown", text: prose }]);
  });

  it("never lifts media out of code, and never lifts a non-http address", () => {
    const fenced = "```\n![x](https://github.com/x.png)\n```";
    expect(splitGithubMedia(fenced)).toEqual([{ kind: "markdown", text: fenced }]);
    expect(media("use `![x](https://github.com/x.png)` to embed")).toEqual([]);
    expect(media('<img src="data:text/html,pwned">\n![x](javascript:alert(1))\n![rel](./docs/shot.png)')).toEqual([]);
  });

  it("reads a tag wrapped across lines", () => {
    expect(media('<img\n  alt="wrapped"\n  src="https://github.com/w.png"\n/>')).toEqual([{ kind: "image", url: "https://github.com/w.png", alt: "wrapped" }]);
  });
});

// Bot comments as they really arrive. Each of these printed raw markup in the
// pane, or blew a sixteen-pixel icon up into a full-width "Load from" row.
describe("bot comments", () => {
  // Verbatim shape of a Vercel deployment comment (state hash shortened).
  const VERCEL = [
    "[vc]: #avn4y/UsDo+ElbHCQCUXnGCGnuvpWh+E37NdEQkZ6f4=:eyJpc01vbm9yZXBvIjp0cnVlfQ==",
    "The latest updates on your projects. Learn more about [Vercel for GitHub](https://vercel.link/github-learn-more).",
    "",
    "| Project | Deployment | Actions | Updated |",
    "| :--- | :----- | :------ | :------ |",
    '| <a href="https://vercel.com/team/bridge-landing"><sup><img src="https://vercel.com/api/www/avatar?projectId=prj_1&s=32" width="16" height="16" align="middle" alt="" /></sup></a> [bridge-landing](https://vercel.com/team/bridge-landing) | ![Ready](https://vercel.com/static/status/ready.svg) [Ready](https://vercel.com/team/bridge-landing/abc) | [Preview](https://bridge-landing-git-x.vercel.app) | <relative-time datetime="2026-11-25T20:49:50.597Z">Nov 25, 2026 8:49pm UTC</relative-time> |',
  ].join("\n");

  it("renders a Vercel comment as its sentence and one clean table row", () => {
    expect(splitGithubMedia(VERCEL).every(block => block.kind === "markdown")).toBe(true);
    const body = normalizeGithubMarkdown(VERCEL);
    expect(body).not.toContain("[vc]");
    expect(body).not.toMatch(/<\/?(?:sup|img|a|relative-time)\b/);
    expect(body.split("\n").at(-1)).toBe("|  [bridge-landing](https://vercel.com/team/bridge-landing) |  [Ready](https://vercel.com/team/bridge-landing/abc) | [Preview](https://bridge-landing-git-x.vercel.app) | Nov 25, 2026 8:49pm UTC |");
  });

  it("writes a badge as what it says and keeps it in its sentence", () => {
    const codex = "**<sub><sub>![P1 Badge](https://img.shields.io/badge/P1-orange?style=flat)</sub></sub>  Guard the list**";
    expect(splitGithubMedia(codex)).toEqual([{ kind: "markdown", text: codex }]);
    expect(normalizeGithubMarkdown(codex)).toBe("**`P1`  Guard the list**");
    expect(normalizeGithubMarkdown('<img src="https://img.shields.io/badge/build-passing-brightgreen" alt="build">')).toBe("`build: passing`");
    expect(normalizeGithubMarkdown("[![CI](https://github.com/o/r/actions/workflows/ci.yml/badge.svg)](https://github.com/o/r/actions)"))
      .toBe("[`CI`](https://github.com/o/r/actions)");
  });

  it("turns Cursor's picture buttons into plain links", () => {
    const cursor = '<p><a href="https://cursor.com/open?data=a" target="_blank" rel="noopener noreferrer"><picture><source media="(prefers-color-scheme: dark)" srcset="https://cursor.com/dark.png"><img alt="Fix in Cursor" width="115" height="28" src="https://cursor.com/dark.png"></picture></a>&nbsp;<a href="https://cursor.com/agents?data=a"><picture><img alt="Fix in Web" width="99" height="28" src="https://cursor.com/web.png"></picture></a></p>';
    expect(splitGithubMedia(cursor).every(block => block.kind === "markdown")).toBe(true);
    expect(normalizeGithubMarkdown(cursor)).toBe("[Fix in Cursor](https://cursor.com/open?data=a) [Fix in Web](https://cursor.com/agents?data=a)");
  });

  it("still lifts a real screenshot, and keeps prose tables whole", () => {
    expect(splitGithubMedia('<img width="900" src="https://github.com/user-attachments/assets/shot.png" alt="pane">')).toEqual([
      { kind: "media", items: [{ kind: "image", url: "https://github.com/user-attachments/assets/shot.png", alt: "pane" }] },
    ]);
    const table = "| Step | Shot |\n|---|---|\n| build | ![s](https://github.com/s.png) |";
    expect(splitGithubMedia(table)).toEqual([{ kind: "markdown", text: table }]);
    expect(normalizeGithubMarkdown(table).split("\n").at(-1)).toBe("| build | [image: s](https://github.com/s.png) |");
  });

  it("decodes entities, expands shortcodes, and unwraps styling tags", () => {
    expect(normalizeGithubMarkdown("a&nbsp;b &mdash; c &amp; &#8594; &#x2713; &bogus;")).toBe("a b — c & → ✓ &bogus;");
    expect(normalizeGithubMarkdown(":white_check_mark: done :not_a_code: 12:30:45")).toBe("✅ done :not_a_code: 12:30:45");
    expect(normalizeGithubMarkdown("<sub>Comment `@coderabbitai help`</sub> H<sup>2</sup> <kbd>⌘</kbd>")).toBe("Comment `@coderabbitai help` H2 `⌘`");
    // An escaped tag decodes to text, after the tag rewrites have run.
    expect(normalizeGithubMarkdown("&lt;b&gt;not bold&lt;/b&gt;")).toBe("<b>not bold</b>");
  });

  it("writes block HTML as the markdown that draws the same thing", () => {
    expect(normalizeGithubMarkdown('<h3 align="center">Steps</h3>\nthen<hr>after')).toBe("### Steps\n\nthen\n\n---\n\nafter");
    expect(normalizeGithubMarkdown("<ul>\n<li>one</li>\n<li>two<ol><li>deep</li></ol></li>\n</ul>")).toBe("- one\n- two\n  1. deep");
    expect(normalizeGithubMarkdown("<blockquote>\n<h2>v1.2</h2>\n<p>Notes</p>\n</blockquote>")).toBe("> ## v1.2\n>\n> Notes");
    expect(normalizeGithubMarkdown("<table><tr><th>Size</th><th>Delta</th></tr><tr><td><code>a.js</code></td><td>+1 | 2</td></tr></table>"))
      .toBe("| Size | Delta |\n| --- | --- |\n| `a.js` | +1 ∣ 2 |");
  });

  it("keeps a table row in one piece when a cell holds a <br>", () => {
    expect(normalizeGithubMarkdown("| File | Summary |\n|---|---|\n| **Renderer** <br> `a.ts` | ok |"))
      .toBe("| File | Summary |\n|---|---|\n| **Renderer**   `a.ts` | ok |");
  });

  it("does not link a reference that is already a link's label", () => {
    expect(normalizeGithubMarkdown('fixed (<a href="https://redirect.github.com/o/r/issues/7">#7</a>)', REPO))
      .toBe("fixed ([#7](https://redirect.github.com/o/r/issues/7))");
  });
});

describe("resolveReferences", () => {
  it("resolves reference links and hides their definitions", () => {
    const body = "See the [design doc][doc], [Rules][] and [faq].\n\n[doc]: https://example.com/design\n[rules]: <https://example.com/rules> \"Rules\"\n[FAQ]: https://example.com/faq";
    expect(resolveReferences(body)).toBe("See the [design doc](https://example.com/design), [Rules](https://example.com/rules) and [faq](https://example.com/faq).\n");
  });

  it("drops a definition whose target is not a link, and leaves undefined labels", () => {
    expect(resolveReferences("[vc]: #R4k+abc=:eyJ0\nText [vc] and [missing].")).toBe("Text vc and [missing].");
  });

  it("marks footnotes, and leaves code alone", () => {
    expect(resolveReferences("Claim[^1].\n\n[^1]: Source.")).toBe("Claim¹.\n\n¹ Source.");
    expect(resolveReferences("`[x]` and\n```\n[x]: kept\n```\n[x]: https://e.test/x")).toBe("`[x]` and\n```\n[x]: kept\n```");
  });

  it("never links a scheme the shell would not open", () => {
    expect(resolveReferences("[click][x]\n\n[x]: javascript:alert(1)")).toBe("click\n");
  });
});

describe("nested sections", () => {
  it("keeps a section inside a section, and honours <details open>", () => {
    const blocks = splitGithubDetails("<details>\n<summary>Outer</summary>\n\n<details>\n<summary>Inner</summary>\n\nbody\n\n</details>\n\n</details>\nafter\n<details open><summary>Shown</summary>x</details>");
    expect(blocks).toEqual([
      { kind: "details", summary: "Outer", body: "<details>\n<summary>Inner</summary>\n\nbody\n\n</details>" },
      { kind: "markdown", text: "\nafter\n" },
      { kind: "details", summary: "Shown", body: "x", open: true },
    ]);
    expect(splitGithubDetails((blocks[0] as { body: string }).body)).toEqual([{ kind: "details", summary: "Inner", body: "body" }]);
  });
});
