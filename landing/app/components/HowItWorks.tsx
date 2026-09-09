import Reveal from "./Reveal";

const steps = [
  {
    title: "Describe the task",
    text: "Bridge opens a task worktree on its own branch and starts an orchestrator session against it. Direct chats work without a repository too.",
  },
  {
    title: "Policy authorizes the workers",
    text: "The orchestrator delegates bounded slices. For each one the policy engine decides: run in the parent, reuse a worker, spawn one, queue it, reject it, or ask you.",
  },
  {
    title: "Verify, then merge",
    text: "Workers return typed results with file evidence. Completion gates can demand a verifier from another harness before the merge waits on your approval.",
  },
];

export default function HowItWorks() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <Reveal>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">
            How a task moves through Bridge
          </h2>
        </Reveal>
        <Reveal className="mt-12 grid gap-10 md:grid-cols-3">
          {steps.map((step, i) => (
            <div key={step.title}>
              <span className="font-mono text-[11px] text-faint">0{i + 1}</span>
              <h3 className="mt-3 text-base font-medium">{step.title}</h3>
              <p className="mt-2 text-[14px] leading-6 text-muted-foreground">{step.text}</p>
            </div>
          ))}
        </Reveal>
      </div>
    </section>
  );
}
