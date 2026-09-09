import { describe, expect, it } from "vitest";
import { normalizeGithubMarkdown, splitGithubDetails } from "./githubMarkdown";

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
    const body = normalizeGithubMarkdown("Fixes #412 for @atharva", REPO);
    expect(body).toBe(`Fixes [#412](${REPO}/issues/412) for [@atharva](https://github.com/atharva)`);
    // No repository context means no dead link.
    expect(normalizeGithubMarkdown("Fixes #412")).toBe("Fixes #412");
    // An email address is not a mention.
    expect(normalizeGithubMarkdown("mail me@example.com")).toContain("me@example.com");
    expect(normalizeGithubMarkdown("mail me@example.com")).not.toContain("github.com/example");
  });

  it("leaves fenced code exactly as authored", () => {
    const source = "before #1\n```ts\nconst x = \"#1 @user https://example.test\";\n```\nafter #1";
    const body = normalizeGithubMarkdown(source, REPO);
    expect(body).toContain('const x = "#1 @user https://example.test";');
    expect(body).toContain(`before [#1](${REPO}/issues/1)`);
    expect(body).toContain(`after [#1](${REPO}/issues/1)`);
  });

  it("leaves inline code spans alone", () => {
    expect(normalizeGithubMarkdown("use `#1` not #1", REPO))
      .toBe(`use \`#1\` not [#1](${REPO}/issues/1)`);
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
});
