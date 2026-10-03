import { useEffect, useRef } from "react";
import type { SlashCommand } from "../types";
import { slashOwnershipBadge } from "../utils";

export function ComposerSlashMenu({ id, commands, index, onIndex, onSelect }: {
  id: string; commands: SlashCommand[]; index: number;
  onIndex: (index: number) => void; onSelect: (command: SlashCommand) => void;
}) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => { list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: "nearest" }); }, [index]);
  return <div id={id} role="listbox" aria-label="Commands and skills" className="u-glass-popover absolute inset-x-0 bottom-full z-20 mb-2 flex max-h-[min(420px,55vh)] flex-col overflow-hidden rounded-2xl">
    <div className="shrink-0 border-b border-border px-3 py-1.5 text-xs text-muted-foreground">Commands & skills</div>
    <div ref={list} className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
      {commands.map((command, row) => <button id={`${id}-option-${row}`} key={`${command.harness}:${command.name}`} role="option" aria-selected={row === index} type="button" onMouseEnter={() => onIndex(row)} onMouseDown={event => { event.preventDefault(); onSelect(command); }} className={`flex w-full items-center gap-2 px-3 py-2 text-left ${row === index ? "bg-accent" : "hover:bg-accent"}`}>
        <span className="shrink-0 font-mono text-xs">/{command.name}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{command.description}</span>
        <span className="shrink-0 text-xs text-muted-foreground">{slashOwnershipBadge(command.harness)}</span>
      </button>)}
    </div>
  </div>;
}
