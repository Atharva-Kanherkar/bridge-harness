import type { PostBlock } from "../content/blog";

export default function PostBody({ blocks }: { blocks: PostBlock[] }) {
  return (
    <div className="mt-10 flex flex-col gap-5">
      {blocks.map((block, i) => {
        switch (block.kind) {
          case "heading":
            return (
              <h2 key={i} className="mt-6 font-display text-xl font-semibold tracking-tight text-foreground">
                {block.text}
              </h2>
            );
          case "paragraph":
            return (
              <p key={i} className="text-[15px] leading-7 text-body">
                {block.text}
              </p>
            );
          case "list":
            return (
              <ul key={i} className="flex flex-col gap-2">
                {block.items.map((item) => (
                  <li key={item} className="flex gap-3 text-[15px] leading-7 text-body">
                    <span className="mt-3 size-1 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />
                    <span>{item}</span>
                  </li>
                ))}
              </ul>
            );
          case "code":
            return (
              <pre
                key={i}
                className="overflow-x-auto rounded-lg border border-border bg-code px-4 py-3 font-mono text-[12.5px] leading-relaxed text-code-foreground"
              >
                {block.code}
              </pre>
            );
          case "aside":
            return (
              <p key={i} className="border-l-2 border-border pl-4 text-[13.5px] leading-6 text-muted-foreground">
                {block.text}
              </p>
            );
        }
      })}
    </div>
  );
}
