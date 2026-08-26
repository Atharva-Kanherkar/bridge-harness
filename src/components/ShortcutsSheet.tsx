import { Keyboard } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { formatChord, SHORTCUT_GROUPS, shortcutsInGroup, type Shortcut } from "../keymap";

// The cheatsheet is rendered from the keymap, never from a second list. A
// shortcut that exists but is not written down here would mean the table and
// the sheet had drifted, which is the thing the table exists to prevent.

function Chord({ shortcut }: { shortcut: Shortcut }) {
  return <kbd className="shrink-0 rounded-md border border-border bg-muted px-1.5 py-0.5 font-mono text-[10.5px] font-medium text-foreground">
    {formatChord(shortcut)}
  </kbd>;
}

export function ShortcutsSheet({ open, onClose }: { open: boolean; onClose: () => void }) {
  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>
    <DialogContent aria-label="Keyboard shortcuts">
      <DialogHeader>
        <div className="flex items-start gap-3">
          <span className="mt-0.5 grid h-8 w-8 shrink-0 place-items-center rounded-xl bg-muted text-foreground"><Keyboard size={16} aria-hidden="true" /></span>
          <div>
            <DialogTitle className="font-display text-base font-medium text-foreground">Keyboard shortcuts</DialogTitle>
            <DialogDescription className="mt-1 text-[11px] leading-5">Every command Bridge answers to. The same bindings sit in the menu bar.</DialogDescription>
          </div>
        </div>
      </DialogHeader>
      <DialogPanel>
        <div className="grid gap-4">
          {SHORTCUT_GROUPS.map(group => <section key={group} aria-label={group}>
            <h3 className="mb-1.5 text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">{group}</h3>
            <ul className="grid gap-0.5">
              {shortcutsInGroup(group).map(shortcut => <li key={shortcut.id} className="flex items-center gap-3 rounded-lg px-1.5 py-1 text-[12px] text-muted-foreground">
                <span className="min-w-0 flex-1 truncate text-foreground">{shortcut.label}</span>
                <Chord shortcut={shortcut} />
              </li>)}
            </ul>
          </section>)}
        </div>
        <p className="mt-4 text-[10.5px] leading-relaxed text-muted-foreground/70">Escape steps back one layer at a time: it restores an expanded dock before it leaves the fullscreen layout.</p>
      </DialogPanel>
    </DialogContent>
  </Dialog>;
}
