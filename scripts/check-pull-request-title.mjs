#!/usr/bin/env node

const TYPES = [
  "build",
  "chore",
  "ci",
  "docs",
  "feat",
  "fix",
  "perf",
  "refactor",
  "revert",
  "style",
  "test",
];

const TYPE = `(?:${TYPES.join("|")})`;
const SCOPE = "(?:\\([a-z0-9][a-z0-9._/-]*\\))?";
const CONVENTIONAL_TITLE = new RegExp(`^${TYPE}${SCOPE}!?: [^\\s].*$`);

export function isConventionalPullRequestTitle(title) {
  return (
    typeof title === "string" &&
    !title.includes("\n") &&
    CONVENTIONAL_TITLE.test(title)
  );
}

export function checkPullRequestTitle(title) {
  if (isConventionalPullRequestTitle(title)) return;
  throw new Error(
    [
      "Pull request titles must use Conventional Commit syntax.",
      "Examples: fix: handle daemon startup race; feat(settings): add provider configuration; feat!: replace the session format.",
      "The squash commit title is Release Please's semantic-version input.",
    ].join("\n"),
  );
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    checkPullRequestTitle(process.argv.slice(2).join(" "));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
