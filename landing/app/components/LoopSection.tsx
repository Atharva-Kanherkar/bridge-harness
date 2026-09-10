import LoopTabs from "./LoopTabs";

export default function LoopSection() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <div>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">Your dev loop, supervised</h2>
          <p className="mt-4 max-w-2xl text-[15px] leading-7 text-muted-foreground">
            The same loop you already run, with a boundary at each step that you can inspect and revoke.
          </p>
        </div>
        <LoopTabs />
      </div>
    </section>
  );
}
