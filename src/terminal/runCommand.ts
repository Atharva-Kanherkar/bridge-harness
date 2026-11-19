// Turning a shell code block an agent wrote into the bytes a shell runs.
// Contract: testing/feat-run-in-terminal.md.

const SHELL_FENCES = new Set(["bash", "sh", "zsh", "shell", "console", "shell-session", "shellsession", "terminal"]);
/** Fences whose convention is a transcript: prompts, then output. */
const SESSION_FENCES = new Set(["console", "shell-session", "shellsession", "terminal"]);

const PROMPT = /^\s*[$%](?: |$)/;
const CONTINUATION_PROMPT = /^\s*> /;
// Every C0/C1 control character except tab and newline. An escape sequence
// hidden in a fence would run without ever being drawn in the block, and an
// embedded end-of-paste marker would break out of bracketed paste.
const CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g;

/** Is this fence's language one a shell runs as written? */
export function isShellFence(lang: string): boolean {
  return SHELL_FENCES.has(lang.trim().toLowerCase());
}

function sanitize(text: string): string {
  return text.replace(CONTROL, "");
}

/** Prompt lines, prompts stripped, plus their `\` continuations. */
function fromTranscript(lines: string[]): string[] {
  const kept: string[] = [];
  let continues = false;
  for (const line of lines) {
    if (PROMPT.test(line)) {
      const command = line.replace(PROMPT, "");
      if (command.trim()) kept.push(command);
      continues = command.trimEnd().endsWith("\\");
    } else if (continues) {
      const command = line.replace(CONTINUATION_PROMPT, "");
      kept.push(command);
      continues = command.trimEnd().endsWith("\\");
    }
  }
  return kept;
}

/** The command a shell fence asks to run, or null when there is none. */
export function commandFromFence(lang: string, body: string): string | null {
  if (!isShellFence(lang)) return null;
  const lines = sanitize(body).split("\n").map(line => line.trimEnd());
  const firstLine = lines.find(line => line.trim()) ?? "";
  const transcript = SESSION_FENCES.has(lang.trim().toLowerCase()) ? lines.some(line => PROMPT.test(line)) : PROMPT.test(firstLine);
  const kept = transcript ? fromTranscript(lines) : lines;
  while (kept.length && !kept[0].trim()) kept.shift();
  while (kept.length && !kept[kept.length - 1].trim()) kept.pop();
  return kept.length ? kept.join("\n") : null;
}

/** One write that types `command` into a shell and presses Enter. When the
 *  shell asked for bracketed paste, a multi-line command arrives as one edit
 *  and Enter runs it whole; otherwise each line runs as it lands. */
export function terminalRunInput(command: string, bracketedPaste: boolean): string {
  const text = sanitize(command).replace(/\r?\n/g, "\r");
  if (bracketedPaste) return `\x1b[200~${text}\x1b[201~\r`;
  // Outside a paste, a tab is the completion key, not whitespace.
  return `${text.replace(/\t/g, " ")}\r`;
}
