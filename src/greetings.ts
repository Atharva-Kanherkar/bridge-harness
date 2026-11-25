// The headline on the empty "new chat" screen and an empty chat. One plain
// question, naming the project when there is one. With a project, the screen
// streams that question in like a model reply and then rotates through the
// other project lines below, in a fixed order rather than a random pick.

export type GreetingPart = { text: string; kind: "text" | "project" };

export interface Greeting {
  /** Fully substituted plain text, for the accessible name and tests. */
  headline: string;
  /** Rich segments for rendering; the project segment is drawn brighter. */
  parts: GreetingPart[];
}

export function welcomeHeadline(project?: string | null): Greeting {
  const named = project?.trim();
  if (!named) return { headline: "What should we build?", parts: [{ text: "What should we build?", kind: "text" }] };
  return {
    headline: `What should we build in ${named}?`,
    parts: [
      { text: "What should we build in ", kind: "text" },
      { text: named, kind: "project" },
      { text: "?", kind: "text" },
    ],
  };
}

/** Lines that end on the project name, so each streamed line lands on it. The
 *  plain question from `welcomeHeadline` comes first. Kept short enough to fit
 *  one row of the hero. */
const PROJECT_LINES: string[] = [
  "What should we build in {project}?",
  "Which bug is quietly haunting {project}?",
  "What would make you proud of {project}?",
  "Point me at the messiest file in {project}.",
  "Which TODO has waited too long in {project}?",
  "What's the 10x version of {project}?",
  "Let's delete some dead code in {project}.",
  "Where are the sharp edges in {project}?",
  "What's the boldest idea for {project}?",
  "Which test is missing from {project}?",
  "Tell me what's slowing down {project}.",
  "What would users love in {project}?",
  "Which flaky test dies today in {project}?",
  "What's the next chapter for {project}?",
  "One small diff to a better {project}.",
  "What's worth shipping today in {project}?",
];

/** Every project line, with the project as its own part. Empty without a project. */
export function projectGreetingLines(project?: string | null): GreetingPart[][] {
  const named = project?.trim();
  if (!named) return [];
  return PROJECT_LINES.map(template => {
    const [before, after] = template.split("{project}");
    return [
      { text: before, kind: "text" as const },
      { text: named, kind: "project" as const },
      ...(after ? [{ text: after, kind: "text" as const }] : []),
    ];
  });
}
