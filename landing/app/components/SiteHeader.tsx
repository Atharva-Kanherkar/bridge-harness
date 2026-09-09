"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { changelogUrl, docsUrl, latestReleaseUrl, repoUrl } from "../content/site";

export default function SiteHeader() {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 8);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  return (
    <header
      className={`sticky top-0 z-50 transition-colors ${scrolled ? "border-b border-border bg-background/80 backdrop-blur-md" : "border-b border-transparent"}`}
    >
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <Link href="/" className="font-display text-[15px] font-semibold tracking-tight">
          Bridge
        </Link>
        <nav className="flex items-center gap-6 text-[13px] text-muted-foreground">
          <a href={docsUrl} className="hover:text-foreground">
            Docs
          </a>
          <a href={changelogUrl} className="hidden hover:text-foreground sm:inline">
            Changelog
          </a>
          <a href={repoUrl} className="hidden hover:text-foreground sm:inline">
            GitHub
          </a>
          <a href={latestReleaseUrl} className="rounded-md bg-foreground px-3 py-1.5 text-background hover:bg-foreground/90">
            Download
          </a>
        </nav>
      </div>
    </header>
  );
}
