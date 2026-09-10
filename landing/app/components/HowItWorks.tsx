import SectionHeader from "./SectionHeader";

const steps = [
  {
    title: "Describe the task",
    text: "Bridge opens a task worktree on its own branch and starts an orchestrator session against it.",
    tag: "task-4c1e · feat/worktree-lifecycle",
  },
  {
    title: "Policy authorizes the workers",
    text: "The orchestrator asks for bounded slices. For each, policy decides: run here, reuse, spawn, queue, reject, or ask you.",
    tag: "standard · isolated · depth 1 of 2",
  },
  {
    title: "Verify, then merge",
    text: "Workers return typed results with file evidence. A gate can demand a second harness before the merge waits on you.",
    tag: "claude implemented · codex verified",
  },
];

export default function HowItWorks() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader eyebrow="How it works" title="A task, from prompt to merge." />
        <ol className="mt-14 grid gap-px overflow-hidden rounded-xl border border-border-card bg-border-card md:grid-cols-3">
          {steps.map((step, i) => (
            <li key={step.title} className="reveal flex flex-col bg-card p-7">
              <span className="font-pixel text-[2.75rem] leading-none text-faint-2">{String(i + 1).padStart(2, "0")}</span>
              <h3 className="mt-6 text-[17px] font-medium text-foreground">{step.title}</h3>
              <p className="mt-2 text-[14px] leading-6 text-muted-foreground">{step.text}</p>
              <span className="mt-6 w-fit rounded-md border border-border bg-code px-2 py-1 font-mono text-[11px] text-faint">{step.tag}</span>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}
