import { Fragment, useMemo } from "react";
import { ChevronRight } from "lucide-react";
import { Markdown } from "../Markdown";
import { normalizeGithubMarkdown, splitGithubDetails, splitGithubMedia } from "../githubMarkdown";
import { MediaRow } from "./Media";

/** A run of GitHub markdown: text through the app's renderer, media through
 *  the pane's own loader. */
function Run({ text, repositoryUrl }: { text: string; repositoryUrl?: string }) {
  const blocks = useMemo(() => splitGithubMedia(text), [text]);
  return <>{blocks.map((block, index) => block.kind === "media"
    ? <MediaRow key={index} items={block.items} />
    : <Fragment key={index}><Markdown text={normalizeGithubMarkdown(block.text, repositoryUrl)} /></Fragment>)}</>;
}

/** GitHub-authored markdown at the pane's compact type scale.
 *
 * Three things happen before the app's renderer sees the text: GitHub-only
 * spellings are normalized (see `githubMarkdown.ts`), `<details>` sections
 * become real disclosures, and images and videos are lifted out into typed
 * media rows. Text still goes through the app's Markdown component, React
 * text nodes only, so remote markup stays inert; media is an `<img>` or a
 * `<video>` with an http(s) address and nothing else from the body. */
export function GithubMarkdown({ text, repositoryUrl }: { text: string; repositoryUrl?: string }) {
  const blocks = useMemo(() => splitGithubDetails(text), [text]);
  return <div className="gh-md">
    {blocks.map((block, index) => block.kind === "details"
      ? <details key={index} className="group my-1.5 overflow-hidden rounded-lg border border-border bg-card">
        <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2.5 py-1.5 text-meta font-medium text-foreground transition-colors hover:bg-accent/50">
          <ChevronRight size={12} aria-hidden="true" className="shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-90" />
          <span className="min-w-0 flex-1 truncate">{block.summary}</span>
        </summary>
        <div className="border-t border-border px-2.5 py-2">
          <Run text={block.body} repositoryUrl={repositoryUrl} />
        </div>
      </details>
      : <Run key={index} text={block.text} repositoryUrl={repositoryUrl} />)}
  </div>;
}
