import type { CloneSettings } from "./types";

export const CLONE_BROWSERS = [
  { value: "chrome", label: "Chrome" },
  { value: "brave", label: "Brave" },
  { value: "safari", label: "Safari (fresh sign-in)" },
];
export const SAFARI_SIGN_IN_HINT = "Safari uses a protected Chromium browser. Sign in here; no Safari cookies are copied.";
export function selectCloneBrowser(settings: CloneSettings, browser: string): CloneSettings {
  return { ...settings, defaultBrowser: browser as CloneSettings["defaultBrowser"], defaultSignInPath: browser === "safari" ? "sign_in_inside" : settings.defaultSignInPath };
}

