import type { Metadata } from "next";
import Link from "next/link";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { posts } from "../content/blog";

export const metadata: Metadata = {
  title: "Blog",
  description: "How Bridge is built: the invariants, the boundaries, and the decisions behind them.",
};

export default function Blog() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-3xl px-6 pb-24 pt-16">
        <h1 className="font-display text-4xl font-semibold tracking-tight sm:text-5xl">Blog</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">
          How Bridge is built: the invariants it refuses to break, and the decisions behind them.
        </p>

        <div className="mt-16 flex flex-col">
          {posts.map((post) => (
            <article key={post.slug} className="border-t border-border py-8 first:border-t-0 first:pt-0">
              <time dateTime={post.iso} className="text-[13px] text-faint">
                {post.date}
              </time>
              <h2 className="mt-2 font-display text-2xl font-semibold tracking-tight">
                <Link href={`/blog/${post.slug}`} className="hover:text-foreground/80">
                  {post.title}
                </Link>
              </h2>
              <p className="mt-3 text-[15px] leading-7 text-muted-foreground">{post.summary}</p>
              <Link
                href={`/blog/${post.slug}`}
                className="mt-4 inline-block text-[13px] text-muted-foreground hover:text-foreground"
              >
                Read it
              </Link>
            </article>
          ))}
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}
