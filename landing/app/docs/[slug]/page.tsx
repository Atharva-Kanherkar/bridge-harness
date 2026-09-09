import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import SiteFooter from "../../components/SiteFooter";
import SiteHeader from "../../components/SiteHeader";
import { docBySlug, docEntries } from "../../content/docs";
import { renderDoc } from "../../lib/renderDoc";
import { repoUrl } from "../../content/site";

export function generateStaticParams() {
  return docEntries.map((entry) => ({ slug: entry.slug }));
}

export async function generateMetadata({ params }: PageProps<"/docs/[slug]">): Promise<Metadata> {
  const { slug } = await params;
  const entry = docBySlug(slug);
  if (!entry) return {};
  return { title: entry.title, description: entry.summary };
}

export default async function DocPage({ params }: PageProps<"/docs/[slug]">) {
  const { slug } = await params;
  const entry = docBySlug(slug);
  if (!entry) notFound();

  const html = await renderDoc(entry.file);

  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-3xl px-6 pb-24 pt-16">
        <Link href="/docs" className="text-[13px] text-muted-foreground hover:text-foreground">
          Docs
        </Link>
        <article className="doc-prose mt-8" dangerouslySetInnerHTML={{ __html: html }} />
        <p className="mt-16 border-t border-border pt-6 text-[13px] text-muted-foreground">
          Source:{" "}
          <a
            href={`${repoUrl}/blob/main/docs/${entry.file}`}
            target="_blank"
            rel="noreferrer"
            className="text-foreground underline underline-offset-4 hover:no-underline"
          >
            docs/{entry.file}
          </a>
        </p>
      </main>
      <SiteFooter />
    </div>
  );
}
