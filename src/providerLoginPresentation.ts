import { isExternalUrl } from "./externalLinks";

// Vendor login commands write terminal control sequences because they believe
// they own a full terminal. The onboarding surface does not: remove the common
// CSI/OSC forms before showing troubleshooting text or searching for a URL.
// The OSC body must stop at its own terminator: BEL, or the two-byte ST
// (`ESC \`). A class that only excludes BEL swallows the ST and every
// character after it, which would eat the vendor's sign-in URL.
const ANSI_SEQUENCE = /\u001B(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001B]*(?:\u0007|\u001B\\)?)/g;
const HTTP_URL = /https?:\/\/[^\s<>"'\u001b]+/gi;

export function plainProviderLoginOutput(output: string): string {
  return output
    .replace(ANSI_SEQUENCE, "")
    .replaceAll("\r", "")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, "")
    .trim();
}

export function providerLoginUrl(output: string): string | null {
  const candidates = plainProviderLoginOutput(output).match(HTTP_URL) ?? [];
  for (const candidate of candidates) {
    const url = candidate.replace(/[),.;]+$/, "");
    if (isExternalUrl(url)) return url;
  }
  return null;
}
