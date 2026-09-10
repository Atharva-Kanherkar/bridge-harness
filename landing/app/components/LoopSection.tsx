import LoopTabs from "./LoopTabs";
import SectionHeader from "./SectionHeader";

export default function LoopSection() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader
          eyebrow="The loop"
          title={
            <>
              Your dev loop, <em className="not-italic text-muted-foreground">supervised.</em>
            </>
          }
          text="The same loop you already run, with a boundary at each step that you can inspect and revoke."
        />
        <LoopTabs />
      </div>
    </section>
  );
}
