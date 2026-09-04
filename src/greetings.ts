// Hardcoded, time-aware greetings for the empty "new chat" screen. A stable
// hash of the session id picks a line so each new chat gets its own prompt that
// doesn't flicker on re-render, while the pool is tinted by the time of day.

export interface Greeting {
  /** Fully-substituted plain text — used for the accessible name and tests. */
  headline: string;
  hint: string;
  /** Rich segments for rendering; the project segment gets the dotted underline. */
  parts: GreetingPart[];
}

export type GreetingPart = { text: string; kind: "text" | "project" };

/** Lines that name the current project. Rendered with a dotted underline under
 *  the `{project}` slot, matching the near-black new-thread mockup. Only used
 *  when a project name is known; otherwise the general/time pools stand in. */
const PROJECT: string[] = [
  "What should we build in {project}?",
  "Where are we headed in {project}?",
  "What's next for {project}?",
  "What are we shipping in {project}?",
  "What are we building in {project}?",
  "What should we improve in {project}?",
  "Where should we start in {project}?",
  "What's the next move on {project}?",
];

const GENERAL: string[] = [
  "What should we build?",
  "What are we making today?",
  "The stage is yours.",
  "What problem are we solving?",
  "Where should we begin?",
  "What's the mission?",
  "Point me at something.",
  "What are we shipping?",
  "Let's make something good.",
  "What's on your mind?",
  "Give me something to build.",
  "What are we creating?",
  "Ready when you are.",
  "What's the task?",
  "Describe the dream.",
  "What are we fixing?",
  "Let's get to work.",
  "What's next on the list?",
  "Hand me the hard part.",
  "What would you build if it were easy?",
  "What's worth building today?",
  "Start anywhere.",
  "What are we improving?",
  "Tell me where it hurts.",
  "What's the idea?",
  "Let's turn thought into code.",
  "What's the plan?",
  "What deserves your attention?",
  "What are we automating?",
  "Bring me a problem.",
  "What's the smallest useful thing?",
  "What's the big swing?",
  "What are we prototyping?",
  "Let's ship something today.",
  "What are we designing?",
  "Where are we headed?",
  "What's the story?",
  "What are we untangling?",
  "What can I take off your plate?",
  "Let's build the thing.",
  "What are we exploring?",
  "What's calling for you?",
  "What's the first move?",
  "Draw me the map.",
  "What are we refactoring?",
  "What's broken that shouldn't be?",
  "Let's make it real.",
  "What are we launching?",
  "What's the experiment?",
  "Say the word.",
  "What are we tackling?",
  "What future are we building?",
  "What's the feature?",
  "Let's begin.",
  "What are we dreaming up?",
  "What should exist that doesn't yet?",
  "Give me the goal.",
  "What are we polishing?",
  "What's the ambition?",
  "Let's make progress.",
  "What are we wiring up?",
  "What's the next milestone?",
  "Whatever it is, let's start.",
  "What are we building for?",
  "What's the itch to scratch?",
  "Let's chase the idea.",
  "What are we bringing to life?",
  "What's the objective?",
  "What are we sketching?",
  "The blank canvas awaits.",
];

const MORNING: string[] = [
  "Good morning. What's first?",
  "Fresh start — what are we building?",
  "Morning. Where do we begin?",
  "A new day, a new build.",
  "Coffee's brewing. What's the plan?",
  "Morning momentum — what's the task?",
  "Let's make today count.",
  "Sunrise ideas — what's on deck?",
  "Early start? Let's build.",
  "First light, first commit.",
  "What are we starting today?",
  "Morning clarity — what's the mission?",
  "Let's open the day with something good.",
];

const AFTERNOON: string[] = [
  "Good afternoon. What's next?",
  "Afternoon momentum — what are we shipping?",
  "Midday build session?",
  "Let's keep it moving.",
  "Good afternoon. Where to?",
  "Post-lunch productivity — what's the task?",
  "The day's still young. What are we making?",
  "Afternoon focus — hand me the work.",
  "Let's make the second half count.",
  "What are we pushing forward?",
  "Steady progress — what's next?",
  "Let's build through the afternoon.",
  "What's on the docket?",
];

const EVENING: string[] = [
  "Good evening. What are we building?",
  "Evening focus. What's the task?",
  "Winding down or winding up?",
  "Golden hour, good ideas.",
  "Good evening. Where do we start?",
  "Evening build session?",
  "Let's close the day with a win.",
  "The evening's yours.",
  "Quiet evening, clear head — what's next?",
  "Let's make something before the day ends.",
  "Evening momentum — what are we shipping?",
  "One more thing before dark?",
  "What are we finishing tonight?",
];

const NIGHT: string[] = [
  "Late night build session?",
  "Burning the midnight oil — what's the task?",
  "The quiet hours are the best hours.",
  "Night owl mode. What are we making?",
  "Can't sleep? Let's build.",
  "The world's asleep. Let's create.",
  "Midnight ideas hit different.",
  "Late and inspired — what's the plan?",
  "The night is yours.",
  "Quiet night, loud ideas.",
  "Still up? Let's ship something.",
  "After hours, all gas.",
  "What are we building under the stars?",
  "Nocturnal productivity — where do we start?",
];

const HINTS: string[] = [
  "Describe the work — Bridge takes it from here.",
  "Pick a model below, then say what you want built.",
  "Say what you want built and Bridge handles the rest.",
  "Ask anything. Switch Claude or Codex anytime.",
  "Describe it in plain words and hit send.",
  "Tell me the outcome you want.",
];

type Bucket = "morning" | "afternoon" | "evening" | "night";

function timeBucket(hour: number): Bucket {
  if (hour >= 5 && hour < 12) return "morning";
  if (hour >= 12 && hour < 17) return "afternoon";
  if (hour >= 17 && hour < 22) return "evening";
  return "night";
}

const BY_BUCKET: Record<Bucket, string[]> = { morning: MORNING, afternoon: AFTERNOON, evening: EVENING, night: NIGHT };

/** FNV-1a — small, stable string hash so a session always shows the same line. */
function hash(value: string): number {
  let h = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    h ^= value.charCodeAt(index);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Split a template around its single `{project}` token into rich segments. */
function toParts(template: string, project: string): GreetingPart[] {
  const [before, after = ""] = template.split("{project}");
  const parts: GreetingPart[] = [
    { text: before, kind: "text" },
    { text: project, kind: "project" },
    { text: after, kind: "text" },
  ];
  return parts.filter(part => part.text.length || part.kind === "project");
}

/**
 * Pick a time-appropriate greeting, stable for a given seed (e.g. session id).
 * When a project name is known, project-titled lines ("What should we build in
 * harness?") join the pool so the hero can name — and dotted-underline — it.
 */
export function pickGreeting(seed: string | undefined, project?: string | null, now: Date = new Date()): Greeting {
  const named = project?.trim();
  const pool = named
    ? [...GENERAL, ...BY_BUCKET[timeBucket(now.getHours())], ...PROJECT]
    : [...GENERAL, ...BY_BUCKET[timeBucket(now.getHours())]];
  const key = seed && seed.length ? seed : `${now.getTime()}-${Math.random()}`;
  const template = pool[hash(key) % pool.length];
  const headline = named ? template.replace("{project}", named) : template;
  const parts = named && template.includes("{project}") ? toParts(template, named) : [{ text: headline, kind: "text" as const }];
  const hint = HINTS[hash(`${key}:hint`) % HINTS.length];
  return { headline, hint, parts };
}
