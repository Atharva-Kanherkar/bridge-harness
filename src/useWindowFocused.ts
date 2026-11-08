import { useEffect, useState } from "react";

/** Whether this window is the one the person is using: focused and visible. */
export function useWindowFocused(): boolean {
  const current = () => document.visibilityState === "visible" && document.hasFocus();
  const [focused, setFocused] = useState(current);
  useEffect(() => {
    const update = () => setFocused(current());
    // During `blur` some engines still report focus, so trust the event.
    const blur = () => setFocused(false);
    window.addEventListener("focus", update);
    window.addEventListener("blur", blur);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return focused;
}
