# Issue format: two audiences, one issue

Every issue in this repository is read by two very different readers, and they
want opposite things.

A **human** wants the shape of the problem in under a minute: what breaks, where
it sits in the architecture, what we decided. Diagrams, not paragraphs.

An **agent** wants everything else: the file paths, the symbols, the wire
shapes, the acceptance criteria, the tests. Verbosity here is a feature — an
agent that has to re-derive the context will re-derive it wrong.

Writing one blended body serves neither. So the rule is mechanical:

> **Every issue must have a `## For humans` section followed by a
> `## For agents` section. An issue without both is closed automatically.**

## The shape

```markdown
## For humans

TL;DR in one or two lines. A diagram. The architecture. The decision and why.
Short enough to read standing up.

## For agents

Reproduction, suspected cause with `file:line`, the files to touch, the
acceptance criteria, the tests to add, what is out of scope. Long.
```

Start from a template in [`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE)
— `bug`, `feature`, or `audit` — and you pass the gate by filling it in.

## What goes where

| | For humans | For agents |
|---|---|---|
| Length | As short as it can be | As long as it needs to be |
| Diagrams | Yes — lead with one | Only if it encodes a contract |
| Code | Rarely | Always, with `file:line` |
| Tone | The conclusion | The instructions |
| Contains | TL;DR, system design, high-level architecture, the decision and its rationale | Repro steps, root cause, files and symbols, wire/protocol changes, acceptance criteria, tests, out-of-scope |
| Does not contain | Line-level implementation detail | Re-explanation of why the work matters |

Diagrams use Bridge's own grammar — a `diagram` fenced block with
`nodes`/`edges`, achromatic except for one accent. Not Mermaid.

## How the gate works

[`.github/workflows/issue-format.yml`](../.github/workflows/issue-format.yml)
runs on every issue `opened`, `edited`, and `reopened`. It calls
[`scripts/issue-format.mjs`](../scripts/issue-format.mjs), which is the single
source of truth for the rule and is covered by
[`scripts/test/issue-format.test.mjs`](../scripts/test/issue-format.test.mjs)
under `bun run test`.

An issue fails if any of these hold:

- there is no `For humans` heading at `##` or `###`,
- there is no `For agents` heading at `##` or `###`,
- either section has fewer than 40 characters of real content — HTML comments,
  empty checkboxes, and `TODO`/`TBD`/`N/A` placeholders do not count,
- `For agents` comes before `For humans`.

On failure the gate comments with exactly what is missing, adds `needs-format`,
and closes the issue as *not planned*. **Nothing is lost** — the body is still
there. Edit it to add the sections and the same workflow reopens the issue and
drops the label. The rejection comment is posted once, not on every edit.

Emoji, `###`, singular wording, and trailing colons are all tolerated
(`### 🤖 For Agent:` passes). Headings deeper than `###` are not — a
sub-subsection cannot satisfy the rule.

## The escape hatch

Some issues genuinely do not fit: a one-line tracking issue, a release
checklist, an automated report. Two ways out:

- add the **`format-exempt`** label, or
- put `<!-- issue-format: exempt -->` in the body (for automation that opens
  issues without label permissions).

Use it rarely. If you are reaching for it on a normal bug or feature, the issue
probably just needs its second section.

## For agents opening issues

`gh issue create` bypasses the templates, so the gate is the only thing
standing between you and an auto-closed issue. Write both sections. Put the
implementation contract in `For agents` and keep `For humans` to a TL;DR, a
diagram, and the architectural framing — the same split you would want if you
were the agent picking the work up cold.
