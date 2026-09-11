import Link from "next/link";

/*
 * The page's one chromatic element, after Uiverse.io by ParasSalunke: a gradient edge with
 * the label and chevron sliding right. The ring is already lit at rest, so the button reads
 * as a control rather than waiting for a cursor to prove it exists; hover brings it to full
 * strength and adds the bloom behind it. The two neutral fills are Bridge's own tokens, so
 * the face sits on true black instead of Tailwind's blue-tinted grays.
 */
const GRADIENT = "bg-linear-to-r from-teal-400 via-blue-500 to-purple-500";

const sizes = {
  sm: { pad: "px-3.5 py-1.5", text: "text-[13px]", icon: "size-4", gap: "gap-1" },
  md: { pad: "px-6 py-3", text: "text-[15px]", icon: "size-5", gap: "gap-2" },
};

export default function GradientButton({
  href,
  label,
  size = "md",
  external = false,
}: {
  href: string;
  label: string;
  size?: keyof typeof sizes;
  external?: boolean;
}) {
  const s = sizes[size];
  const inner = (
    <>
      <span aria-hidden="true" className={`absolute -inset-1 rounded-2xl ${GRADIENT} opacity-0 blur-lg transition-opacity duration-500 group-hover:opacity-40`} />
      <span aria-hidden="true" className={`absolute inset-0 rounded-xl ${GRADIENT} opacity-50 transition-opacity duration-500 group-hover:opacity-100`} />
      <span className={`relative z-10 block rounded-xl bg-background ${s.pad}`}>
        <span className={`relative z-10 flex items-center ${s.gap}`}>
          <span className={`${s.text} transition-transform duration-500 group-hover:translate-x-0.5`}>{label}</span>
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
