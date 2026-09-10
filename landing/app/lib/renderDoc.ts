import { readFile } from "node:fs/promises";
import path from "node:path";
import { marked } from "marked";
import { docByFile } from "../content/docs";
import { repoUrl } from "../content/site";

const docsRoot = path.join(process.cwd(), "..", "docs");
const blobBase = `${repoUrl}/blob/main/docs`;

function resolveHref(href: string, fromFile: string): string {
  if (/^(https?:|mailto:|#)/.test(href)) return href;

  const [target, hash] = href.split("#");
  if (!target) return href;

  const fromDir = path.posix.dirname(fromFile);
  const normalized = path.posix.normalize(path.posix.join(fromDir === "." ? "" : fromDir, target));
  const entry = docByFile(normalized);
  if (entry) return `/docs/${entry.slug}${hash ? `#${hash}` : ""}`;

  return `${blobBase}/${normalized}${hash ? `#${hash}` : ""}`;
}

export async function renderDoc(file: string): Promise<string> {
  const source = await readFile(path.join(docsRoot, file), "utf8");
  const renderer = new marked.Renderer();
  const linkRenderer = renderer.link.bind(renderer);

  renderer.link = (token) => {
    const html = linkRenderer({ ...token, href: resolveHref(token.href, file) });
    return resolveHref(token.href, file).startsWith("/docs/")
      ? html
      : html.replace("<a ", '<a target="_blank" rel="noreferrer" ');
  };

  return marked.parse(source, { renderer, async: false, gfm: true });
}
