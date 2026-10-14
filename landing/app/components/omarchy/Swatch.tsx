import type { OmarchySiteTheme } from "../../content/omarchyThemes";

/* A theme's ground with its accent set in it, the way Omarchy's theme picker
 * previews one. The colours are data from the theme files, so they ride a
 * custom property rather than a utility. */
export default function Swatch({ theme, size = "sm" }: { theme: OmarchySiteTheme; size?: "sm" | "md" }) {
  return (
    <span
      aria-hidden="true"
      style={{ "--sw-bg": theme.colors.background, "--sw-fg": theme.colors.accent, "--sw-ink": theme.colors.foreground } as React.CSSProperties}
      className={`relative grid shrink-0 place-items-center border border-(color:--sw-ink)/25 bg-(color:--sw-bg) ${size === "md" ? "size-7" : "size-4"}`}
    >
      <span className={`bg-(color:--sw-fg) ${size === "md" ? "size-2.5" : "size-1.5"}`} />
    </span>
  );
}
