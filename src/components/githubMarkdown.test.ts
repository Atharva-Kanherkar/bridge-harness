import { describe, expect, it } from "vitest";
import { normalizeGithubMarkdown, splitGithubDetails, splitGithubMedia } from "./githubMarkdown";

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
