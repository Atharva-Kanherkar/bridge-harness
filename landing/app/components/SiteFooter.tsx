import Link from "next/link";
import { blogPath, changelogPath, docsPath, downloadPath, issuesUrl, latestVersion, platformLabel, repoUrl } from "../content/site";

const columns = [
  {
    title: "Product",
    links: [
      { label: "Download", href: downloadPath },
      { label: "Changelog", href: changelogPath },
      { label: "Docs", href: docsPath },
      { label: "Blog", href: blogPath },
    ],
  },
  {
    title: "Project",
    links: [
      { label: "GitHub", href: repoUrl },
      { label: "Issues", href: issuesUrl },
    ],
  },
];

export default function SiteFooter() {
  return (
    <footer className="border-t border-border">
      <div className="mx-auto grid max-w-6xl gap-10 px-6 py-14 sm:grid-cols-3">
        <div>
          <span className="font-display text-[15px] font-semibold text-foreground">Bridge</span>
          <p className="mt-2 max-w-xs text-[13px] leading-6 text-muted-foreground">
            A native macOS control room for supervised coding-agent work.
          </p>
        </div>
        {columns.map((column) => (
          <nav key={column.title} className="text-[13px]">
            <h3 className="text-[11px] uppercase tracking-wider text-faint">{column.title}</h3>
            <ul className="mt-3 flex flex-col gap-2 text-muted-foreground">
              {column.links.map((link) => (
                <li key={link.label}>
                  {link.href.startsWith("/") ? (
                    <Link href={link.href} className="hover:text-foreground">
                      {link.label}
                    </Link>
                  ) : (
                    <a href={link.href} className="hover:text-foreground">
                      {link.label}
                    </a>
                  )}
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </div>
      <div className="mx-auto max-w-6xl border-t border-border px-6 py-6 text-[12px] text-faint">
        v{latestVersion} · {platformLabel}
      </div>
    </footer>
  );
}
