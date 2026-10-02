import Link from "next/link";

/*
 * After Uiverse.io by ParasSalunke, achromatic. Two variants share one shape and label:
 *
 * - `primary` is a solid block, `bg-foreground` on `text-background`, the same treatment as
 *   the platform buttons on /download. It is the one call to action in a group, so it reads
 *   first without depending on a hover state or a lit edge.
 * - `outline` is the original: a lit edge that sweeps brighter across the middle, with the
 *   label and chevron sliding right. The ring is already lit at rest, so the button reads as
 *   a control rather than waiting for a cursor to prove it exists.
 *
 * The label wears Bridge's own pixel face, the one the wordmark is set in.
 */
const EDGE = "bg-linear-to-r from-foreground/35 via-foreground/80 to-foreground/35";

const sizes = {
  sm: { pad: "px-3.5 py-1.5", text: "text-[12px]", icon: "size-4", gap: "gap-1.5" },
  md: { pad: "px-6 py-3", text: "text-[15px]", icon: "size-5", gap: "gap-2.5" },
};

export type ActionButtonVariant = "primary" | "outline";

export default function ActionButton({
  href,
  label,
  size = "md",
  variant = "outline",
  external = false,
}: {
  href: string;
  label: string;
  size?: keyof typeof sizes;
  variant?: ActionButtonVariant;
  external?: boolean;
}) {
  const s = sizes[size];
  const primary = variant === "primary";
  const inner = (
    <>
      <span aria-hidden="true" className={`absolute -inset-1 rounded-2xl ${EDGE} opacity-0 blur-lg transition-opacity duration-500 group-hover:opacity-25`} />
      {!primary && <span aria-hidden="true" className={`absolute inset-0 rounded-xl ${EDGE} opacity-55 transition-opacity duration-500 group-hover:opacity-100`} />}
      <span className={`relative z-10 block rounded-xl ${primary ? "bg-foreground text-background group-hover:bg-foreground/90" : "bg-background"} ${s.pad}`}>
        <span className={`relative z-10 flex items-center ${s.gap}`}>
          <span className={`${s.text} font-pixel uppercase tracking-[0.1em] transition-transform duration-500 group-hover:translate-x-0.5`}>{label}</span>
          <svg
            className={`${s.icon} transition-transform duration-500 group-hover:translate-x-1`}
            aria-hidden="true"
            fill="currentColor"
            viewBox="0 0 20 20"
            xmlns="http://www.w3.org/2000/svg"
          >
            <path
              clipRule="evenodd"
              d="M8.22 5.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L11.94 10 8.22 6.28a.75.75 0 0 1 0-1.06Z"
              fillRule="evenodd"
            />
          </svg>
        </span>
      </span>
    </>
  );

  const className =
    "relative inline-block rounded-xl p-px font-semibold leading-6 text-foreground shadow-lg shadow-black/60 transition-transform duration-300 ease-in-out hover:scale-[1.03] active:scale-95";

  return (
    <span className="group relative inline-block">
      {external ? (
        <a href={href} className={className}>
          {inner}
        </a>
      ) : (
        <Link href={href} className={className}>
          {inner}
        </Link>
      )}
    </span>
  );
}
