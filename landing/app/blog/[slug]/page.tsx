import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import PostBody from "../../components/PostBody";
import SiteFooter from "../../components/SiteFooter";
import SiteHeader from "../../components/SiteHeader";
import { postBySlug, posts } from "../../content/blog";

export function generateStaticParams() {
  return posts.map((post) => ({ slug: post.slug }));
}

export async function generateMetadata({ params }: PageProps<"/blog/[slug]">): Promise<Metadata> {
  const { slug } = await params;
  const post = postBySlug(slug);
  if (!post) return {};
  return { title: post.title, description: post.summary };
}

export default async function BlogPost({ params }: PageProps<"/blog/[slug]">) {
  const { slug } = await params;
  const post = postBySlug(slug);
  if (!post) notFound();

  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-2xl px-6 pb-24 pt-16">
        <Link href="/blog" className="text-[13px] text-muted-foreground hover:text-foreground">
          Blog
        </Link>
        <time dateTime={post.iso} className="mt-8 block text-[13px] text-faint">
          {post.date}
        </time>
        <h1 className="mt-2 font-display text-3xl font-semibold leading-tight tracking-tight sm:text-4xl">{post.title}</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">{post.summary}</p>
        <PostBody blocks={post.blocks} />
      </main>
      <SiteFooter />
    </div>
  );
}
