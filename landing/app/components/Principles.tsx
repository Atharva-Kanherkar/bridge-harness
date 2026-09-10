import SectionHeader from "./SectionHeader";

const rules = [
  {
    title: "The three hierarchies stay separate.",
    text: "Workspace tree, agent tree, conversation tree. Forking a conversation does not undo files, and ending a session does not discard a worktree. Bridge surfaces divergence instead of hiding it.",
  },
  {
    title: "Learning may rank. Only policy may grant.",
    text: "Routing and adaptive learning order the candidates the policy engine already allows. They never grant a permission, widen a scope, or skip an approval.",
  },
  {
    title: "History is appended, never rewritten.",
    text: "Compaction keeps the original events and adds a verified checkpoint. A resumed session reports how context actually came back.",
  },
];

export default function Principles() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader eyebrow="Principles" title="Three rules the code keeps." text="Most of Bridge follows from refusing to blur three distinctions." />
        <div className="mt-14 grid gap-10 md:grid-cols-3">
          {rules.map((rule, i) => (
            <div key={rule.title} className="reveal">
              <span className="font-mono text-[11px] text-faint">0{i + 1}</span>
              <h3 className="mt-4 font-display text-[1.375rem] font-semibold leading-[1.2] tracking-[-0.02em] text-foreground">{rule.title}</h3>
              <p className="mt-4 text-[14px] leading-6 text-muted-foreground">{rule.text}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
