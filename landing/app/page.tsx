import Link from "next/link";
import CallToAction from "./components/CallToAction";
import Capabilities from "./components/Capabilities";
import Comparison from "./components/Comparison";
import Faq from "./components/Faq";
import HarnessSection from "./components/HarnessSection";
import HeroBackdrop from "./components/HeroBackdrop";
import AppDemo from "./components/AppDemo";
import HowItWorks from "./components/HowItWorks";
import LoopSection from "./components/LoopSection";
import Principles from "./components/Principles";
import SiteFooter from "./components/SiteFooter";
import SiteHeader from "./components/SiteHeader";
import { downloadPath } from "./content/site";

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
              Supervise a team of coding agents
              <br className="hidden sm:inline" /> from one window.
            </h1>

            <div style={{ "--i": 2 } as React.CSSProperties} className="enter mt-6 flex flex-col items-center justify-center gap-3 sm:flex-row">
              <Link
                href={downloadPath}
                className="group inline-flex h-10 w-[220px] items-center justify-between rounded-md bg-foreground pl-4 pr-3 text-[14px] font-medium text-background transition-colors hover:bg-foreground/90"
              >
                Get Bridge
                <span aria-hidden="true" className="transition-transform group-hover:translate-x-0.5">
                  →
                </span>
              </Link>
            </div>
          </div>

          <div style={{ "--i": 3 } as React.CSSProperties} className="enter mt-10 sm:mt-12">
            <AppDemo />
          </div>
        </div>
      </main>

      <HowItWorks />
      <HarnessSection />
      <LoopSection />
      <Capabilities />
      <Principles />
      <Comparison />
      <Faq />
      <CallToAction />
      <SiteFooter />
    </div>
  );
}
