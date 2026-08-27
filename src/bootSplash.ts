const HIDE_CLASS = "boot-splash-hide";
const REMOVE_FALLBACK_MS = 400;

/**
 * Fades out and removes the static `#boot-splash` node from `index.html` once
 * the app has actually painted, covering exactly the webview-boot +
 * hydration gap the splash exists for (#370). Waits two animation frames so
 * the fade starts after a real paint rather than racing it. Safe to call more
 * than once, and a no-op if the node isn't present (e.g. a test harness that
 * doesn't load the real `index.html`).
 */
export function dismissBootSplash(): void {
  const splash = document.getElementById("boot-splash");
  if (!splash) return;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    splash.classList.add(HIDE_CLASS);
    let removed = false;
    const remove = () => {
      if (removed) return;
      removed = true;
      splash.remove();
    };
    splash.addEventListener("transitionend", remove, { once: true });
    setTimeout(remove, REMOVE_FALLBACK_MS);
  }));
}
