import FeatureScroll from "./components/FeatureScroll";
import Faq from "./components/Faq";
import HarnessGrid from "./components/HarnessGrid";
import HeroBackdrop from "./components/HeroBackdrop";
import AppDemo from "./components/AppDemo";
import ActionButton from "./components/ActionButton";
import { downloadPath } from "./content/site";
import SiteFooter from "./components/SiteFooter";
import SiteHeader from "./components/SiteHeader";

export default function Home() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />

      <main className="relative overflow-hidden border-b border-border">
        <HeroBackdrop />

        <div className="relative w-full px-4 pb-16 pt-10 sm:px-6 sm:pb-20 sm:pt-14">
          <div className="mx-auto w-full max-w-[780px] text-center">
            <h1
              style={{ "--i": 0 } as React.CSSProperties}
              className="enter font-display text-[2rem] font-semibold leading-[1.08] tracking-[-0.03em] text-foreground sm:text-[2.75rem]"
            >
              The last coding agent
              <br className="hidden sm:inline" /> you&rsquo;ll ever need.
            </h1>

            <p style={{ "--i": 1 } as React.CSSProperties} className="enter mx-auto mt-5 max-w-2xl text-[15.5px] leading-7 text-muted-foreground">
              Claude Code, Codex, Cursor, and OpenCode, running as one team in one window.
            </p>

            <div style={{ "--i": 2 } as React.CSSProperties} className="enter mt-8 flex justify-center">
              <ActionButton href={downloadPath} label="Get Bridge" />
            </div>
          </div>

          <div style={{ "--i": 3 } as React.CSSProperties} className="enter mt-10 sm:mt-12">
            <AppDemo />
          </div>
        </div>
      </main>

      <HarnessGrid />
      <FeatureScroll />
      <Faq />
      <SiteFooter />
    </div>
  );
}
