import { useCallback, useEffect, useRef, useState } from "react";
import { SkeletonDot, SkeletonRegion, SkeletonText } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { CodeEditor } from "./CodeEditor";
import { isDirty, isReadOnly, loadBuffer, saveBuffer, settleSave, stateAfterEdit, statusLabel, type FileBuffer } from "./fileBuffer";
import { modKey } from "@/lib/platform";

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

  // The latest buffer for callbacks fired between renders (⌘S and a click in
  // one tick), and the write in flight; see CodePanel's `saving`.
  const latest = useRef(buffer);
  latest.current = buffer;
  const saving = useRef(false);
  const live = useRef(true);
  useEffect(() => () => { live.current = false; }, []);

  const save = useCallback(async (force = false) => {
    const current = latest.current;
    if (!current || isReadOnly(current) || saving.current) return;
    if (current.state === "error" && !current.baseSha) return;
    const text = content.current;
    if (!force && current.state === "clean" && text === current.saved) return;
    saving.current = true;
    setBuffer({ ...current, state: "saving", message: undefined });
    try {
      const next = await saveBuffer(workspaceId, current, text, force);
      if (next.state === "clean") onSaved?.();
      // Text typed while the write was in flight is still unsaved.
      if (live.current) setBuffer(settleSave(next, content.current));
    } finally {
      saving.current = false;
    }
  }, [onSaved, workspaceId]);

  const reload = useCallback(async () => {
    const seed = (latest.current?.seed ?? 0) + 1;
    const loaded = await loadBuffer(workspaceId, path, seed);
    if (!live.current) return;
    content.current = loaded.saved;
    setBuffer(loaded);
  }, [path, workspaceId]);

  if (!buffer) return <SkeletonRegion label={`Opening ${path}`} className="px-3.5 py-4"><SkeletonText lines={6} /></SkeletonRegion>;

  if (buffer.state === "error" && !buffer.baseSha) {
    return <p className="px-3.5 py-4 text-meta leading-relaxed text-destructive">{buffer.message}</p>;
  }
  if (isReadOnly(buffer)) {
    return <p className="px-3.5 py-4 text-meta leading-relaxed text-muted-foreground">
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
            return next && current ? { ...current, state: next, message: next === "clean" ? undefined : current.message } : current;
          });
        }}
        onSave={() => void save()}
        className="cm-fit"
      />
    </div>
    <div className="flex min-h-9 flex-wrap items-center gap-2 py-1 border-t border-border px-3 font-mono text-caption text-muted-foreground">
      {buffer.state === "saving" && <SkeletonDot className="size-[11px]" />}
      <span className={cn(
        buffer.state === "conflict" && "text-warning",
        buffer.state === "error" && "text-destructive",
        buffer.state === "dirty" && "text-foreground",
      )}>{statusLabel(buffer)}</span>
      <span className="ml-auto flex items-center gap-2">
        {buffer.state === "conflict" && <>
          <button type="button" onClick={() => void reload()} className="min-h-7 rounded-md px-2 underline decoration-dotted underline-offset-2 hover:text-foreground">Reload</button>
          <button type="button" onClick={() => void save(true)} className="min-h-7 rounded-md px-2 underline decoration-dotted underline-offset-2 hover:text-foreground">Overwrite</button>
        </>}
        {buffer.state !== "conflict" && <button
          type="button"
          onClick={() => void save()}
          disabled={buffer.state === "clean" || buffer.state === "saving"}
          className="min-h-7 rounded-md px-2 underline decoration-dotted underline-offset-2 hover:text-foreground disabled:no-underline disabled:opacity-45"
        >Save {modKey("S")}</button>}
      </span>
    </div>
  </div>;
}
