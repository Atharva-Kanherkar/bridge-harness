
const rules = [
  {
    title: "The three hierarchies stay separate",
    text: "The workspace tree, the agent tree, and the conversation tree are related but independent. Forking a conversation does not undo filesystem changes, and ending a session does not discard a worktree. Conflating them is the recurring design bug, so Bridge surfaces divergence instead of hiding it.",
  },
  {
    title: "Learning may rank. Only policy may grant",
    text: "Routing and adaptive learning can order the candidates the policy engine has already found eligible. They can never grant a permission, widen a write scope, or skip an approval. Every safety gate has exactly one owner.",
  },
  {
    title: "History is appended, never rewritten",
    text: "Compaction preserves the original events and adds a verified checkpoint boundary. A resumed session reports how context actually came back rather than implying continuity it does not have.",
  },
];

export default function Principles() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <div>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">Three rules the code keeps</h2>
          <p className="mt-4 max-w-2xl text-[15px] leading-7 text-muted-foreground">
            Most of Bridge follows from refusing to blur three distinctions.
          </p>
        </div>
        <div className="mt-12 grid gap-8 md:grid-cols-3">
          {rules.map((rule, i) => (
            <div key={rule.title} className="border-t border-border pt-5">
              <span className="font-mono text-[11px] text-faint">0{i + 1}</span>
              <h3 className="mt-3 text-base font-medium leading-6">{rule.title}</h3>
              <p className="mt-3 text-[14px] leading-6 text-muted-foreground">{rule.text}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
