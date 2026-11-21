// The headline on the empty "new chat" screen and an empty chat. One plain
// question, naming the project when there is one. It used to roll a random,
// time-of-day line per chat; that read as noise on the first screen.

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
