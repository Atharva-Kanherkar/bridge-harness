import { Fragment, useMemo } from "react";
import { ChevronRight } from "lucide-react";
import { Markdown } from "../Markdown";
import { normalizeGithubMarkdown, resolveReferences, splitGithubDetails, splitGithubMedia } from "../githubMarkdown";
import { MediaRow } from "./Media";

/** A run of GitHub markdown: text through the app's renderer, media through
 *  the pane's own loader. */
function Run({ text, repositoryUrl }: { text: string; repositoryUrl?: string }) {
  const blocks = useMemo(() => splitGithubMedia(text), [text]);
  return <>{blocks.map((block, index) => block.kind === "media"
    ? <MediaRow key={index} items={block.items} />
    : <Fragment key={index}><Markdown text={normalizeGithubMarkdown(block.text, repositoryUrl)} /></Fragment>)}</>;
}

/** Prose and `<details>` sections, each section's body split again so a
 *  section inside a section is a disclosure too. */
function Sections({ text, repositoryUrl, depth }: { text: string; repositoryUrl?: string; depth: number }) {
  const blocks = useMemo(() => depth < 6 ? splitGithubDetails(text) : [{ kind: "markdown" as const, text }], [text, depth]);
  return <>{blocks.map((block, index) => block.kind === "details"
    ? <details key={index} open={block.open} className="my-1.5 overflow-hidden rounded-lg border border-border bg-card">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2.5 py-1.5 text-meta font-medium text-foreground transition-colors hover:bg-accent/50">
        <ChevronRight size={12} aria-hidden="true" className="shrink-0 text-muted-foreground transition-transform duration-200 [details[open]>summary>&]:rotate-90" />
        <span className="min-w-0 flex-1 truncate">{block.summary}</span>
      </summary>
      <div className="border-t border-border px-2.5 py-2">
        <Sections text={block.body} repositoryUrl={repositoryUrl} depth={depth + 1} />
      </div>
    </details>
    : <Run key={index} text={block.text} repositoryUrl={repositoryUrl} />)}</>;
}

/** GitHub-authored markdown at the pane's compact type scale.
 *
 * Four things happen before the app's renderer sees the text: reference
 * links and footnotes are resolved against the whole body, `<details>`
 * sections become real disclosures, images and videos are lifted out into
 * typed media rows, and GitHub-only spellings are normalized (see
 * `githubMarkdown.ts`). Text still goes through the app's Markdown component,
 * React text nodes only, so remote markup stays inert; media is an `<img>` or
 * a `<video>` with an http(s) address and nothing else from the body. */
export function GithubMarkdown({ text, repositoryUrl }: { text: string; repositoryUrl?: string }) {
  const resolved = useMemo(() => resolveReferences(text), [text]);
  return <div className="gh-md">
    <Sections text={resolved} repositoryUrl={repositoryUrl} depth={0} />
  </div>;
}
