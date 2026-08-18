import type { LanguageSupport } from "@codemirror/language";

/**
 * Resolve a CodeMirror language for a path, loaded on demand.
 *
 * Every grammar is a dynamic import: opening a TypeScript file must not also
 * pay for the PHP parser. The map is keyed by the same names
 * `languageFromPath` produces for the diff viewer, so the editor and the diff
 * never disagree about what language a file is in.
 */
type Loader = () => Promise<LanguageSupport>;

/** Wrap a legacy stream mode as a `LanguageSupport`. */
const stream = async (load: () => Promise<Record<string, unknown>>, member: string): Promise<LanguageSupport> => {
  const [{ StreamLanguage, LanguageSupport: Support }, modes] = await Promise.all([import("@codemirror/language"), load()]);
  return new Support(StreamLanguage.define(modes[member] as Parameters<typeof StreamLanguage.define>[0]));
};

const shell = () => import("@codemirror/legacy-modes/mode/shell");
const ruby = () => import("@codemirror/legacy-modes/mode/ruby");
const clike = () => import("@codemirror/legacy-modes/mode/clike");

const LOADERS: Record<string, Loader> = {
  typescript: async () => (await import("@codemirror/lang-javascript")).javascript({ typescript: true, jsx: true }),
  javascript: async () => (await import("@codemirror/lang-javascript")).javascript({ jsx: true }),
  rust: async () => (await import("@codemirror/lang-rust")).rust(),
  python: async () => (await import("@codemirror/lang-python")).python(),
  json: async () => (await import("@codemirror/lang-json")).json(),
  css: async () => (await import("@codemirror/lang-css")).css(),
  scss: async () => (await import("@codemirror/lang-css")).css(),
  xml: async () => (await import("@codemirror/lang-html")).html(),
  markdown: async () => (await import("@codemirror/lang-markdown")).markdown(),
  yaml: async () => (await import("@codemirror/lang-yaml")).yaml(),
  sql: async () => (await import("@codemirror/lang-sql")).sql(),
  go: async () => (await import("@codemirror/lang-go")).go(),
  cpp: async () => (await import("@codemirror/lang-cpp")).cpp(),
  c: async () => (await import("@codemirror/lang-cpp")).cpp(),
  java: async () => (await import("@codemirror/lang-java")).java(),
  php: async () => (await import("@codemirror/lang-php")).php(),
  // Stream grammars: less precise than a Lezer parser, but they are the
  // difference between coloured code and a wall of grey.
  bash: () => stream(shell, "shell"),
  ruby: () => stream(ruby, "ruby"),
  swift: () => stream(() => import("@codemirror/legacy-modes/mode/swift"), "swift"),
  lua: () => stream(() => import("@codemirror/legacy-modes/mode/lua"), "lua"),
  perl: () => stream(() => import("@codemirror/legacy-modes/mode/perl"), "perl"),
  ini: () => stream(() => import("@codemirror/legacy-modes/mode/toml"), "toml"),
  dockerfile: () => stream(() => import("@codemirror/legacy-modes/mode/dockerfile"), "dockerFile"),
  makefile: () => stream(() => import("@codemirror/legacy-modes/mode/cmake"), "cmake"),
  csharp: () => stream(clike, "csharp"),
  kotlin: () => stream(clike, "kotlin"),
  scala: () => stream(clike, "scala"),
  objectivec: () => stream(clike, "objectiveC"),
  dart: () => stream(clike, "dart"),
  elixir: () => stream(ruby, "ruby"),
  protobuf: () => stream(() => import("@codemirror/legacy-modes/mode/protobuf"), "protobuf"),
};

/** The language names the editor can highlight, for tests and diagnostics. */
export const EDITOR_LANGUAGES = Object.keys(LOADERS);

/**
 * Load the grammar for a normalized language name, or `null` when we have
 * none — the editor still opens, just without colour.
 */
export async function loadLanguage(language: string): Promise<LanguageSupport | null> {
  const loader = LOADERS[language];
  if (!loader) return null;
  try {
    return await loader();
  } catch {
    return null;
  }
}
