import { useEffect, useState } from "react";
import { FlaskConical, PencilLine } from "lucide-react";
import { bridgeApi } from "../../api";
import type { WorkflowTemplate } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { CHIP, ErrorNote, errorText } from "./parts";
import { triggerSummary } from "./format";
import { SkeletonCards } from "@/components/ui/skeleton";

const capitalized = (app: string) => app.replace(/\b\w/g, letter => letter.toUpperCase());

/** Why a template cannot be tried yet, or null. */
export function templateBlocker(template: WorkflowTemplate): string | null {
  if (template.missing.length > 0) return `Connect ${template.missing.map(capitalized).join(" and ")} in Claude first.`;
  if (template.needsProject) return "Choose a project with Customize.";
  return null;
}

/**
 * Ready-made workflows. Each is a draft: Try it saves it unapproved and runs
 * it dry (every write is a preview), Customize opens it in the editor. Nothing
 * runs for real until the person approves it.
 */
export function WorkflowTemplates({ busy, onTry, onCustomize }: {
  busy: boolean;
  onTry: (template: WorkflowTemplate) => void;
  onCustomize: (template: WorkflowTemplate) => void;
}) {
  const [templates, setTemplates] = useState<WorkflowTemplate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    bridgeApi.listWorkflowTemplates()
      .then(list => { if (current) setTemplates(list.templates); })
      .catch(reason => { if (current) { setError(errorText(reason)); setTemplates([]); } });
    return () => { current = false; };
  }, []);

  if (templates === null) {
    return <SkeletonCards label="Loading templates" count={6} className="mt-4" cardClassName="h-32" />;
  }
  return <div className="mt-4">
    <p className="mb-3 text-caption text-muted-foreground">Try one first: it runs as a dry run, so every change it would make is only shown to you. Nothing runs for real until you approve it.</p>
    <div className="@container"><div className="grid gap-3 @min-[520px]:grid-cols-2">{templates.map(template => {
      const blocker = templateBlocker(template);
      return <article key={template.id} aria-label={template.name} className="flex flex-col rounded-xl border border-border bg-card p-4">
        <h3 className="text-ui font-semibold text-foreground">{template.name}</h3>
        <p className="mt-1 flex-1 text-meta leading-relaxed text-muted-foreground">{template.description}</p>
        <div className="mt-3 flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
          <span className={CHIP}>{triggerSummary(template.definition.trigger)}</span>
          {template.needs.map(app => <span key={app} className={CHIP}>{capitalized(app)}, read only</span>)}
          {template.needsProject && <span className={CHIP}>a project</span>}
        </div>
        {blocker && <p className="mt-2 text-caption text-muted-foreground">{blocker}</p>}
        <div className="mt-3 flex flex-wrap gap-1.5">
          <Button size="xs" disabled={busy || !template.ready} onClick={() => onTry(template)}><FlaskConical size={11}/>Try it (dry run)</Button>
          <Button size="xs" variant="secondary" disabled={busy} onClick={() => onCustomize(template)}><PencilLine size={11}/>Customize</Button>
        </div>
      </article>;
    })}</div></div>
    <ErrorNote message={error}/>
  </div>;
}
