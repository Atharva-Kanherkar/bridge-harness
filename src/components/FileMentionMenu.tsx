import type { Ref } from "react";
import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import type { ChatFileChange } from "../chatChangedFiles";
import { markSegments, mentionMatch, splitMentionPath, type MatchRange } from "../fileMentionRows";
import { glyphFor } from "./fileGlyph";

/** Another composer trigger, taught in the footer: `#` agents, `/` commands. */
export interface MentionTrigger {
  key: string;
  label: string;
}

/** The main composer's other triggers, as its placeholder names them. */
export const COMPOSER_MENTION_TRIGGERS: readonly MentionTrigger[] = [
  { key: "#", label: "agents" },
  { key: "/", label: "commands" },
  { key: "$", label: "provider" },
];

/** A side chat is pinned to its harness and dispatches no agents: only `/`. */
export const SIDE_CHAT_MENTION_TRIGGERS: readonly MentionTrigger[] = [
  { key: "/", label: "commands" },
];

const CHANGE_WORDS: Record<ChatFileChange, string> = { A: "added in this chat", M: "changed in this chat" };
const CHANGE_INK: Record<ChatFileChange, string> = { A: "text-success", M: "text-info" };

/**
 * The composer's `@` file menu. The main composer and the side chat share it,
 * so the two can never draw a file differently.
 *
 * Purely presentational. The host owns the query, the ranked matches, the
 * active row and every key; this draws them and reports hover and pick.
 */
export function FileMentionMenu({ id, optionIdPrefix, files, query, activeIndex, changes, triggers = [], listRef, className, onActiveIndexChange, onPick }: {
  /** The listbox id the composer's `aria-controls` names. */
  id: string;
  /** Each row's id is this plus its index, for `aria-activedescendant`. */
  optionIdPrefix: string;
  /** Already ranked and capped by the host. */
  files: readonly string[];
  query: string;
  activeIndex: number;
  /** Files this chat has written to, keyed by workspace-relative path. */
  changes?: ReadonlyMap<string, ChatFileChange>;
  triggers?: readonly MentionTrigger[];
  listRef?: Ref<HTMLDivElement>;
  /** Placement and height, which differ between hosts. */
  className?: string;
  onActiveIndexChange: (index: number) => void;
  onPick: (path: string) => void;
}) {
  const marks = new Set(files.map(file => changes?.get(file)).filter((mark): mark is ChatFileChange => !!mark));
  return <div id={id} role="listbox" aria-label="Files" className={cn("u-glass-popover z-20 flex flex-col overflow-hidden rounded-2xl", className)}>
    <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5 text-caption font-medium text-muted-foreground/70">
      <span>Files</span>
      {query && <span className="min-w-0 truncate text-muted-foreground/50">matching “{query}”</span>}
      <span className="tabular-nums text-muted-foreground/50">{files.length}</span>
      {marks.size > 0 && <span className="ml-auto inline-flex shrink-0 items-center gap-1 font-normal" data-mention-legend>
        {(["M", "A"] as const).filter(mark => marks.has(mark)).map(mark => <span key={mark} className={`font-mono ${CHANGE_INK[mark]}`}>{mark}</span>)}
        <span>changed in this chat</span>
      </span>}
    </div>
    <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain" onWheel={event => event.stopPropagation()}>
      {files.map((file, index) => <FileMentionRow
        key={file}
        id={`${optionIdPrefix}${index}`}
        index={index}
        path={file}
        query={query}
        active={index === activeIndex}
        change={changes?.get(file)}
        onHover={onActiveIndexChange}
        onPick={onPick}
      />)}
    </div>
    <div aria-hidden="true" className="flex shrink-0 items-center gap-4 border-t border-border bg-muted/30 px-3 py-1.5 text-caption text-muted-foreground">
      <span className="flex items-center gap-1.5"><Kbd>↵</Kbd>Insert</span>
      {triggers.map(trigger => <span key={trigger.key} className="flex items-center gap-1.5"><Kbd>{trigger.key}</Kbd>{trigger.label}</span>)}
    </div>
  </div>;
}

function FileMentionRow({ id, index, path, query, active, change, onHover, onPick }: {
  id: string;
  index: number;
  path: string;
  query: string;
  active: boolean;
  change?: ChatFileChange;
  onHover: (index: number) => void;
  onPick: (path: string) => void;
}) {
  const { name, folder } = splitMentionPath(path);
  const match = mentionMatch(path, query);
  const { Icon } = glyphFor(path);
  return <button
    id={id}
    role="option"
    aria-selected={active}
    aria-label={change ? `${path}, ${CHANGE_WORDS[change]}` : path}
    title={path}
    type="button"
    data-mention-index={index}
    onMouseEnter={() => onHover(index)}
    onMouseDown={event => { event.preventDefault(); onPick(path); }}
    className={`flex min-h-11 w-full items-center gap-2 px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${active ? "bg-accent" : "hover:bg-accent"}`}
  >
    <Icon size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
    <span data-mention-name className="min-w-0 max-w-[70%] shrink-0 truncate font-mono text-meta text-foreground"><Marked text={name} range={match.name} /></span>
    {folder && <span data-mention-folder className="min-w-0 flex-1 truncate text-caption text-faint"><Marked text={folder} range={match.folder} /></span>}
    {change && <span data-mention-change={change} className={`ml-auto shrink-0 pl-2 font-mono text-caption ${CHANGE_INK[change]}`} title={CHANGE_WORDS[change]}>{change}</span>}
  </button>;
}

/** The matched run underlined and weighted, never coloured: the row's ink stays its own. */
function Marked({ text, range }: { text: string; range?: MatchRange }) {
  return <>{markSegments(text, range).map((segment, index) => segment.marked
    ? <mark key={index} className="bg-transparent font-semibold text-inherit underline decoration-muted-foreground/50 underline-offset-2">{segment.text}</mark>
    : <span key={index}>{segment.text}</span>)}</>;
}
