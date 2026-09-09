import { tagHighlighter, tags as t, type Tag } from "@lezer/highlight";

/**
 * The editor's syntax highlighter.
 *
 * This replaces `@lezer/highlight`'s stock `classHighlighter`, which was
 * quietly losing most of a file's colour. `classHighlighter` maps only 29
 * class names, and a Lezer tag that has no spec of its own resolves through
 * its `set` to whatever base tag *does* — so on the stock highlighter:
 *
 *   - `function(variableName)` and `function(propertyName)` fall back to
 *     `tok-variableName`/`tok-propertyName`, meaning **no function call or
 *     declaration is ever distinguishable from a plain identifier**;
 *   - `tagName` falls back to `tok-typeName`, so JSX and HTML element names
 *     wear the type colour;
 *   - `attributeName` falls back to `tok-propertyName`;
 *   - `regexp`, `escape` and `special(string)` collapse into `tok-string2`;
 *   - `inserted`/`deleted` emit classes the app never styled at all.
 *
 * `index.css` had rules for `.tok-function`, `.tok-tagName`,
 * `.tok-attributeName`, `.tok-regexp` and a dozen more siblings. None of them
 * could ever match. Fixing the palette without fixing this would have repainted
 * rules that never fire.
 *
 * So the mapping is owned here, and it emits the *same* `stx-*` buckets the
 * Shiki scope classifier emits for chat code and diffs (see
 * `SYNTAX_CLASSES`). One class vocabulary, one palette, three renderers — the
 * property the old `classHighlighter` comment claimed and the class-name
 * mismatch silently broke.
 */
const SPECS: { tag: Tag | Tag[]; class: string }[] = [
  // Comments. `lineComment`/`blockComment`/`docComment` all carry `comment`
  // in their set, so one spec is the whole family.
  { tag: t.comment, class: "stx-comment" },

  // Keywords. Same story: `controlKeyword`, `moduleKeyword`,
  // `definitionKeyword` and `operatorKeyword` resolve through `keyword`.
  { tag: t.keyword, class: "stx-keyword" },
  { tag: t.modifier, class: "stx-keyword" },
  // `this`/`self` is a keyword wearing an identifier's clothes; without this
  // spec its set resolves to `keyword` anyway, but stating it keeps the
  // intent legible next to `variableName` below.
  { tag: t.self, class: "stx-keyword" },

  // Literals. Booleans, atoms, `null` and units are keyword-adjacent in
  // Lezer's taxonomy but read as values, and the Shiki classifier already
  // buckets `constant.language` as a number — matching it here is what stops
  // `true` being violet in the editor and amber in chat.
  { tag: t.string, class: "stx-string" },
  { tag: t.attributeValue, class: "stx-string" },
  { tag: [t.regexp, t.escape, t.special(t.string)], class: "stx-regex" },
  { tag: [t.number, t.bool, t.atom, t.null, t.unit, t.literal, t.color], class: "stx-number" },
  // No `constant(variableName)` spec on purpose. It is tempting — a
  // SCREAMING_CASE name is arguably a literal — but the Shiki side cannot
  // match it: TextMate gives `variable.other.constant` to every `const`
  // binding, so treating "constant" as a value there painted most of a
  // TypeScript file amber. Both renderers leave it as an identifier so the
  // two never disagree.

  // Names. `function(...)` is listed for both bases because a method call
  // arrives as `function(propertyName)` and a bare call as
  // `function(variableName)`; a declaration composes `definition` on top and
  // still resolves to one of these two.
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], class: "stx-function" },
  { tag: [t.typeName, t.className, t.namespace], class: "stx-type" },
  { tag: [t.tagName, t.standard(t.tagName)], class: "stx-tag" },
  { tag: [t.attributeName, t.propertyName, t.labelName], class: "stx-property" },
  // Last of the identifier family on purpose: `definition(variableName)`,
  // `local(variableName)` and `special(variableName)` have no spec of their
  // own and land here, which is right — a declared name is still a name.
  { tag: t.variableName, class: "stx-variable" },

  // Operators carry meaning that punctuation does not, so they get their own
  // hue. `derefOperator` is the exception: a `.` reads as structure, and the
  // Shiki side buckets `punctuation.accessor` as punctuation.
  { tag: t.operator, class: "stx-operator" },
  // `=>` is tagged `function(punctuation)`, which resolves through
  // `punctuation` and would come out grey. The Shiki side reaches the same
  // decision from `storage.type.function.arrow`.
  { tag: t.function(t.punctuation), class: "stx-operator" },
  { tag: t.derefOperator, class: "stx-punct" },
  // `bracket`, `brace`, `paren`, `squareBracket`, `angleBracket` and
  // `separator` all resolve through `punctuation`.
  { tag: t.punctuation, class: "stx-punct" },

  // Markup and diagnostics.
  { tag: t.meta, class: "stx-meta" },
  { tag: t.invalid, class: "stx-invalid" },
  { tag: [t.link, t.url], class: "stx-link" },
  { tag: t.heading, class: "stx-heading" },
  { tag: t.emphasis, class: "stx-emphasis" },
  { tag: t.strong, class: "stx-strong" },
  { tag: t.strikethrough, class: "stx-strike" },
  // Inline code in a markdown buffer. The Shiki side buckets
  // `markup.inline.raw` the same way, so a fenced-off `npm install` reads
  // identically in the Code tab and in a chat message.
  { tag: t.monospace, class: "stx-string" },
  // A `.diff`/`.patch` buffer opened in the Code tab. The stock highlighter
  // emitted `tok-inserted`/`tok-deleted` and nothing styled them.
  { tag: t.inserted, class: "stx-addition" },
  { tag: t.deleted, class: "stx-deletion" },
];

/** Every class this highlighter can emit. `palette.test.ts` asserts this stays
 *  a subset of `SYNTAX_CLASSES` and that each entry has a rule in
 *  `index.css` — the check that makes it impossible to add a bucket here and
 *  forget its colour. */
export const EDITOR_SYNTAX_CLASSES: string[] = [...new Set(SPECS.map(spec => spec.class))];

export const bridgeHighlighter = tagHighlighter(SPECS);
