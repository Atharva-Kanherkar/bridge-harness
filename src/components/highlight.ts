import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("c", c);
hljs.registerLanguage("cpp", cpp);
hljs.registerLanguage("css", css);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("go", go);
hljs.registerLanguage("ini", ini);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("swift", swift);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("yaml", yaml);

const LANG_ALIASES: Record<string, string> = {
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  py: "python", rs: "rust", sh: "bash", zsh: "bash", shell: "bash", console: "bash",
  yml: "yaml", html: "xml", vue: "xml", svelte: "xml", md: "markdown",
  "c++": "cpp", objc: "c", patch: "diff", toml: "ini", plaintext: "", text: "",
};

export function normalizeLang(lang: string): string {
  const key = lang.trim().toLowerCase();
  if (!key) return "";
  const aliased = LANG_ALIASES[key] ?? key;
  return hljs.getLanguage(aliased) ? aliased : "";
}

/** Highlight a code block, returning safe HTML (hljs escapes entities). */
export function highlightCode(code: string, lang: string): string {
  const language = normalizeLang(lang);
  try {
    if (language) return hljs.highlight(code, { language, ignoreIllegals: true }).value;
    return hljs.highlightAuto(code).value;
  } catch {
    return escapeHtml(code);
  }
}

/** Highlight a unified diff, returning safe HTML with addition/deletion classes. */
export function highlightDiff(code: string): string {
  try {
    return hljs.highlight(code, { language: "diff", ignoreIllegals: true }).value;
  } catch {
    return escapeHtml(code);
  }
}

const DIFF_LINE = /^(\+\+\+|---|@@|\+[^+]|-[^-])/m;

/** Heuristic: does this tool output look like a unified diff/patch? */
export function looksLikeDiff(text: string): boolean {
  if (!text) return false;
  const sample = text.slice(0, 4000);
  if (/^@@ /m.test(sample)) return true;
  if (/^(diff --git|--- |\+\+\+ )/m.test(sample)) return true;
  const lines = sample.split("\n").filter(line => line.length > 0);
  if (lines.length < 3) return false;
  const diffLines = lines.filter(line => DIFF_LINE.test(line));
  return diffLines.length >= 3 && diffLines.length / lines.length > 0.4;
}

export function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
