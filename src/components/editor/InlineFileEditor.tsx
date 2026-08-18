import { useCallback, useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { CodeEditor } from "./CodeEditor";
import { isDirty, isReadOnly, loadBuffer, saveBuffer, stateAfterEdit, statusLabel, type FileBuffer } from "./fileBuffer";

/**
 * One file from the Changes tab, opened for editing in place.
 *
 * Reviewing a diff and wanting to fix the thing you just read is the same
 * motion, so the fix happens here rather than in another tab. The write goes
 * through the same hash guard as the editor: an agent's concurrent edit turns
 * into a choice, never a silent overwrite.
 */
export function InlineFileEditor({ workspaceId, path, onDirtyChange, onSaved }: {
  workspaceId: string;
  path: string;
  /** Lifted so the row can keep this editor mounted while it holds unsaved
   *  text, and say so on the collapsed row. */
  onDirtyChange?: (dirty: boolean) => void;
  onSaved?: () => void;
}) {
  const [buffer, setBuffer] = useState<FileBuffer>();
  // The live document lives outside React so typing re-renders nothing.
  const content = useRef("");
  const dirty = buffer ? isDirty(buffer) : false;
  useEffect(() => { onDirtyChange?.(dirty); }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  useEffect(() => {
    let live = true;
    void loadBuffer(workspaceId, path).then(loaded => {
      if (!live) return;
      content.current = loaded.saved;
      setBuffer(loaded);
    });
    return () => { live = false; };
  }, [path, workspaceId]);

  const save = useCallback(async (force = false) => {
    const current = buffer;
    if (!current || isReadOnly(current) || current.state === "saving") return;
    setBuffer({ ...current, state: "saving", message: undefined });
    const next = await saveBuffer(workspaceId, current, content.current, force);
    setBuffer(next);
    if (next.state === "clean") onSaved?.();
  }, [buffer, onSaved, workspaceId]);

  const reload = useCallback(async () => {
    const seed = (buffer?.seed ?? 0) + 1;
    const loaded = await loadBuffer(workspaceId, path, seed);
    content.current = loaded.saved;
    setBuffer(loaded);
  }, [buffer?.seed, path, workspaceId]);

  if (!buffer) return <div className="flex items-center gap-2 px-3.5 py-4 text-[11.5px] text-muted-foreground">
    <LoaderCircle size={12} className="animate-spin" aria-hidden="true" /> Opening {path}…
  </div>;

  if (buffer.state === "error" && !buffer.baseSha) {
    return <p className="px-3.5 py-4 text-[11.5px] leading-relaxed text-destructive">{buffer.message}</p>;
  }
  if (isReadOnly(buffer)) {
    return <p className="px-3.5 py-4 text-[11.5px] leading-relaxed text-muted-foreground">
      {buffer.tooLarge ? `${Math.round(buffer.sizeBytes / 1024)} KB — too large to open in the editor.` : "Binary file — nothing to edit here."}
    </p>;
  }

  return <div>
    <div>
      <CodeEditor
        key={`${path}:${buffer.seed}`}
        docKey={`${path}:${buffer.seed}`}
        doc={buffer.saved}
        path={path}
        onChange={value => {
          content.current = value;
          setBuffer(current => {
            const next = current && stateAfterEdit(current, value);
            return next && current ? { ...current, state: next } : current;
          });
        }}
        onSave={() => void save()}
        className="cm-fit"
      />
    </div>
    <div className="flex h-[28px] items-center gap-2 border-t border-border px-3 font-mono text-[10.5px] text-muted-foreground">
      {buffer.state === "saving" && <LoaderCircle size={11} className="animate-spin" aria-hidden="true" />}
      <span className={cn(
        buffer.state === "conflict" && "text-warning",
        buffer.state === "error" && "text-destructive",
        buffer.state === "dirty" && "text-foreground",
      )}>{statusLabel(buffer)}</span>
      <span className="ml-auto flex items-center gap-2">
        {buffer.state === "conflict" && <>
          <button type="button" onClick={() => void reload()} className="underline decoration-dotted underline-offset-2 hover:text-foreground">Reload</button>
          <button type="button" onClick={() => void save(true)} className="underline decoration-dotted underline-offset-2 hover:text-foreground">Overwrite</button>
        </>}
        {buffer.state !== "conflict" && <button
          type="button"
          onClick={() => void save()}
          disabled={buffer.state === "clean" || buffer.state === "saving"}
          className="underline decoration-dotted underline-offset-2 hover:text-foreground disabled:no-underline disabled:opacity-45"
        >Save ⌘S</button>}
      </span>
    </div>
  </div>;
}
