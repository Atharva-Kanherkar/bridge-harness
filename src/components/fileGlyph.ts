import {
  Binary, Braces, Coffee, Container, Database, FileCode, FileDiff, FileJson, FileLock,
  FileTerminal, FileText, FileType, Gem, GitBranch, Hash, Image, Package, Palette,
  Settings, Table, type LucideIcon,
} from "lucide-react";
import { languageFromPath } from "./highlight";

/** A file's glyph and its tint class. */
export interface FileGlyph {
  Icon: LucideIcon;
  /** A `text-syn-*` utility — always a syntax-ramp token, never a chrome
   *  colour and never a raw hex value, so the tree tints from the same ramp
   *  the code inside the file is painted with. */
  tint: string;
}

/**
 * Language → glyph. Keyed by `languageFromPath`'s canonical vocabulary so the
 * tree and the highlighter can never disagree about what a file is: if the
 * glyph says TypeScript, the buffer really will be tokenized as TypeScript.
 *
 * Tints group by family rather than giving all thirty-odd languages their own
 * hue — script (blue), systems (amber/orange), data/markup (green), style
 * (magenta), shell and config (punctuation grey). Six hues is enough to tell
 * a directory of `.ts` from a directory of `.rs` at a glance, and few enough
 * that a wide tree does not read as confetti.
 */
const BY_LANGUAGE: Partial<Record<string, FileGlyph>> = {
  typescript: { Icon: FileCode, tint: "text-syn-function" },
  javascript: { Icon: FileCode, tint: "text-syn-number" },
  python: { Icon: FileCode, tint: "text-syn-string" },
  rust: { Icon: FileCode, tint: "text-syn-operator" },
  go: { Icon: FileCode, tint: "text-syn-tag" },
  ruby: { Icon: Gem, tint: "text-syn-operator" },
  java: { Icon: Coffee, tint: "text-syn-operator" },
  kotlin: { Icon: Coffee, tint: "text-syn-keyword" },
  swift: { Icon: FileCode, tint: "text-syn-number" },
  c: { Icon: FileCode, tint: "text-syn-property" },
  cpp: { Icon: FileCode, tint: "text-syn-property" },
  csharp: { Icon: FileCode, tint: "text-syn-keyword" },
  objectivec: { Icon: FileCode, tint: "text-syn-property" },
  dart: { Icon: FileCode, tint: "text-syn-tag" },
  elixir: { Icon: FileCode, tint: "text-syn-keyword" },
  scala: { Icon: FileCode, tint: "text-syn-operator" },
  php: { Icon: FileCode, tint: "text-syn-keyword" },
  lua: { Icon: FileCode, tint: "text-syn-function" },
  perl: { Icon: FileCode, tint: "text-syn-property" },

  json: { Icon: FileJson, tint: "text-syn-number" },
  yaml: { Icon: FileText, tint: "text-syn-string" },
  ini: { Icon: Settings, tint: "text-syn-punct" },
  xml: { Icon: FileType, tint: "text-syn-tag" },
  graphql: { Icon: Braces, tint: "text-syn-type" },
  protobuf: { Icon: Braces, tint: "text-syn-tag" },
  sql: { Icon: Database, tint: "text-syn-tag" },

  css: { Icon: Palette, tint: "text-syn-type" },
  scss: { Icon: Palette, tint: "text-syn-type" },

  markdown: { Icon: FileText, tint: "text-syn-punct" },
  diff: { Icon: FileDiff, tint: "text-syn-comment" },

  bash: { Icon: FileTerminal, tint: "text-syn-punct" },
  makefile: { Icon: FileTerminal, tint: "text-syn-punct" },
  dockerfile: { Icon: Container, tint: "text-syn-tag" },
};

/**
 * Whole-filename conventions, checked before the language map.
 *
 * These are files whose *role* is more informative than their syntax: a lock
 * file is JSON but you never read it, `.gitignore` has no grammar at all, and
 * `package.json` is the one JSON file in a repo you actually open. Matching
 * the ordering in `languageFromPath`, this pass runs first so a dotfile is
 * not mistaken for an extension.
 */
const BY_NAME: Record<string, FileGlyph> = {
  "package.json": { Icon: Package, tint: "text-syn-string" },
  "cargo.toml": { Icon: Package, tint: "text-syn-operator" },
  "bun.lock": { Icon: FileLock, tint: "text-syn-comment" },
  "bun.lockb": { Icon: FileLock, tint: "text-syn-comment" },
  "package-lock.json": { Icon: FileLock, tint: "text-syn-comment" },
  "cargo.lock": { Icon: FileLock, tint: "text-syn-comment" },
  "yarn.lock": { Icon: FileLock, tint: "text-syn-comment" },
  ".gitignore": { Icon: GitBranch, tint: "text-syn-comment" },
  ".gitattributes": { Icon: GitBranch, tint: "text-syn-comment" },
  ".gitmodules": { Icon: GitBranch, tint: "text-syn-comment" },
};

/** Extensions with no grammar but an obvious role. */
const BY_EXTENSION: Record<string, FileGlyph> = {
  png: { Icon: Image, tint: "text-syn-type" },
  jpg: { Icon: Image, tint: "text-syn-type" },
  jpeg: { Icon: Image, tint: "text-syn-type" },
  gif: { Icon: Image, tint: "text-syn-type" },
  webp: { Icon: Image, tint: "text-syn-type" },
  svg: { Icon: Image, tint: "text-syn-type" },
  ico: { Icon: Image, tint: "text-syn-type" },
  woff: { Icon: FileType, tint: "text-syn-punct" },
  woff2: { Icon: FileType, tint: "text-syn-punct" },
  ttf: { Icon: FileType, tint: "text-syn-punct" },
  otf: { Icon: FileType, tint: "text-syn-punct" },
  csv: { Icon: Table, tint: "text-syn-string" },
  tsv: { Icon: Table, tint: "text-syn-string" },
  bin: { Icon: Binary, tint: "text-syn-comment" },
  wasm: { Icon: Binary, tint: "text-syn-keyword" },
  snap: { Icon: Hash, tint: "text-syn-comment" },
};

/** Anything unrecognized: a plain sheet in the quietest tint on the ramp. */
const FALLBACK: FileGlyph = { Icon: FileText, tint: "text-syn-punct" };

/**
 * The glyph and tint for a file path. Pure, so the tree can call it per row
 * during render without a memo.
 */
export function glyphFor(path: string): FileGlyph {
  const name = path.split(/[\\/]/).pop()?.toLowerCase() ?? "";
  const byName = BY_NAME[name];
  if (byName) return byName;
  // Language before extension: `languageFromPath` already resolves aliases
  // (`.tsx` → typescript) and layered extensions (`config.yaml.tmpl`), so
  // going through it keeps one detection path instead of two that drift.
  const byLanguage = BY_LANGUAGE[languageFromPath(path)];
  if (byLanguage) return byLanguage;
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const byExtension = BY_EXTENSION[name.slice(dot + 1)];
    if (byExtension) return byExtension;
  }
  return FALLBACK;
}
