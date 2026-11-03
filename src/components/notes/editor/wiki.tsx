// The `[[` menu: existing pages filtered as you type, and a choice to make
// the page when none has the title yet, the way Notion's link menu works.

import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Extension, type Editor, type Range } from "@tiptap/core";
import Suggestion, { type SuggestionKeyDownProps, type SuggestionProps } from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";
import { FileText, Plus } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

export interface WikiPage {
  id: string;
  title: string;
  /** Another page wins `[[title]]`, so this one is linked by its id. */
  byId?: boolean;
}

/** What a page link needs from the page around the editor. */
export interface WikiHost {
  pages: () => WikiPage[];
  createByTitle?: (title: string) => Promise<{ id: string; title: string } | null>;
}

export interface WikiChoice {
  target: string;
  /** The page an existing choice is, for titles `[[…]]` cannot hold. */
  id?: string;
  byId?: boolean;
  /** Make the page at `target` before linking to it. */
  create: boolean;
}

const WIKI_SHOWN = 20;

/** The first pages matching every word of the query as typed, plus one
 *  Create choice when nothing has the exact title (case and spacing aside). */
export function matchWikiChoices(pages: WikiPage[], query: string): WikiChoice[] {
  const clean = query.trim().toLowerCase();
  const words = clean.split(/\s+/).filter(Boolean);
  const found = pages.filter(page => {
    const title = page.title.toLowerCase();
    return words.every(word => title.includes(word));
  });
  const choices: WikiChoice[] = found.slice(0, WIKI_SHOWN).map(page => ({ target: page.title, id: page.id, byId: page.byId, create: false }));
  if (clean && !pages.some(page => page.title.trim().toLowerCase() === clean)) choices.push({ target: query.trim(), create: true });
  return choices;
}

export interface WikiState {
  query: string;
  range: Range;
  rect: DOMRect | null;
  /** The pages and the Create choice matching the query. */
  items: WikiChoice[];
  choose: (choice: WikiChoice) => void;
}

export const wikiKey = new PluginKey("noteWiki");

/** A title `[[…]]` can carry and read back as itself. */
export function wikiSafe(title: string): boolean {
  return !!title.trim() && !/[[\]|\r\n]/.test(title);
}

/** The `[[` trigger. The menu itself is React, fed through `onState`. */
export function wikiCommand(onState: (state: WikiState | null) => void, onKey: (event: KeyboardEvent) => boolean, host: () => WikiHost) {
  return Extension.create({
    name: "noteWiki",
    addProseMirrorPlugins() {
      return [Suggestion<WikiChoice, WikiChoice>({
        editor: this.editor,
        pluginKey: wikiKey,
        char: "[[",
        allowSpaces: true,
        startOfLine: false,
        allow: ({ state, range }) => {
          const $from = state.doc.resolve(range.from);
          // A bracket or bar past the `[[` ends the link being typed: the
          // menu lets go there rather than catch Enter for the whole line.
          const typed = state.doc.textBetween(range.from, range.to).slice(2);
          // Code is never a link, a block or a span of it.
          const inCode = $from.parent.type.spec.code || $from.marks().some(mark => mark.type.spec.code);
          return !/[[\]|]/.test(typed) && !inCode && $from.parent.type.name !== "detailsSummary";
        },
        items: ({ query }) => matchWikiChoices(host().pages(), query),
        command: ({ editor, range, props }) => {
          editor.chain().focus().deleteRange(range).run();
          // A page `[[…]]` would not reach (a title it misreads, or one an
          // older page holds) is linked by its id instead.
          if (props.id && (props.byId || !wikiSafe(props.target))) {
            editor.chain().focus().insertContentAt(range.from, { type: "pageLink", attrs: { id: props.id, title: props.target } }).run();
            return;
          }
          editor.chain().focus().insertContentAt(range.from, { type: "wikiLink", attrs: { target: props.target, alias: null } }).run();
          if (props.create) void host().createByTitle?.(props.target);
        },
        render: () => {
          const show = (props: SuggestionProps<WikiChoice, WikiChoice>) => onState({
            query: props.query, range: props.range, rect: props.clientRect?.() ?? null, items: props.items,
            choose: choice => props.command(choice),
          });
          return {
            onStart: show,
            onUpdate: show,
            onKeyDown: ({ event }: SuggestionKeyDownProps) => {
              if (event.key === "Escape") { onState(null); return true; }
              return onKey(event);
            },
            onExit: () => onState(null),
          };
        },
      })];
    },
  });
}

export interface WikiMenuHandle { onKeyDown: (event: KeyboardEvent) => boolean }

export const WikiMenu = forwardRef<WikiMenuHandle, { state: WikiState }>(function WikiMenu({ state }, ref) {
  const choices = state.items;
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setActive(0), [state]);
  useEffect(() => { list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" }); }, [active]);
  useImperativeHandle(ref, () => ({
    onKeyDown: event => {
      if (!choices.length) return false;
      if (event.key === "ArrowDown") { setActive(index => (index + 1) % choices.length); return true; }
      if (event.key === "ArrowUp") { setActive(index => (index - 1 + choices.length) % choices.length); return true; }
      if (event.key === "Enter" || event.key === "Tab") { state.choose(choices[Math.min(active, choices.length - 1)]); return true; }
      return false;
    },
  }), [choices, active, state]);
  const rect = state.rect;
  if (!rect) return null;
  const below = rect.bottom + 260 < window.innerHeight;
  // In the body: a transformed pane would make `fixed` relative to itself.
  return createPortal(<div
    ref={list}
    role="listbox"
    aria-label="Page links"
    style={{ left: Math.min(rect.left, window.innerWidth - 300), top: below ? rect.bottom + 6 : undefined, bottom: below ? undefined : window.innerHeight - rect.top + 6 }}
    className="fixed z-50 max-h-64 w-72 overflow-y-auto rounded-xl bg-popover p-1 shadow-lg ring-1 ring-border"
    onMouseDown={event => event.preventDefault()}
  >
    {choices.length
      ? choices.map((choice, index) => <button
        key={choice.id ?? `make-${choice.target}`}
        type="button"
        role="option"
        data-index={index}
        aria-selected={index === active}
        onMouseEnter={() => setActive(index)}
        onClick={() => state.choose(choice)}
        className={clsx("flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left", index === active ? "bg-accent" : "")}
      >
        <span className="grid size-7 shrink-0 place-items-center rounded-md bg-card text-muted-foreground ring-1 ring-border">
          {choice.create ? <Plus size={15} strokeWidth={1.75} aria-hidden="true" /> : <FileText size={15} strokeWidth={1.75} aria-hidden="true" />}
        </span>
        <span className="min-w-0 truncate text-[13px] text-foreground">{choice.create ? `Make "${choice.target}"` : choice.target}</span>
      </button>)
      : <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">No pages match “{state.query}”.</p>}
  </div>, document.body);
});
