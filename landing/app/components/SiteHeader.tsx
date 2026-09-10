"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { blogPath, changelogPath, docsPath, downloadPath } from "../content/site";

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
        <Link href="/" className="font-pixel text-[20px] leading-none tracking-[0.04em] uppercase">
          Bridge
        </Link>
        <nav className="flex items-center gap-6 text-[13px] text-muted-foreground">
          <Link href={docsPath} className="hover:text-foreground">
            Docs
          </Link>
          <Link href={changelogPath} className="hidden hover:text-foreground sm:inline">
            Changelog
          </Link>
          <Link href={blogPath} className="hidden hover:text-foreground sm:inline">
            Blog
          </Link>
          <Link href={downloadPath} className="rounded-full bg-foreground px-3.5 py-1.5 text-background transition-colors hover:bg-foreground/90">
            Download
          </Link>
        </nav>
      </div>
    </header>
  );
}
