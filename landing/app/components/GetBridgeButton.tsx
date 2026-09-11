import Link from "next/link";
import { downloadPath } from "../content/site";

/*
 * The one chromatic element on the page: a gradient ring that fades in on hover, from
 * Uiverse.io by ParasSalunke. The two neutral fills are Bridge's own tokens rather than
 * Tailwind's grays, so the button sits on true black instead of a blue-tinted near-black.
 */
export default function GetBridgeButton() {
  return (
    <div className="group relative inline-block">
      <Link
        href={downloadPath}
        className="relative inline-block rounded-xl bg-card p-px font-semibold leading-6 text-foreground shadow-2xl shadow-black transition-transform duration-300 ease-in-out hover:scale-105 active:scale-95"
      >
        <span
          aria-hidden="true"
          className="absolute inset-0 rounded-xl bg-linear-to-r from-teal-400 via-blue-500 to-purple-500 p-[2px] opacity-0 transition-opacity duration-500 group-hover:opacity-100"
        />
        <span className="relative z-10 block rounded-xl bg-background px-6 py-3">
          <span className="relative z-10 flex items-center space-x-2">
            <span className="text-[15px] transition-all duration-500 group-hover:translate-x-1">Get Bridge</span>
            <svg
              className="size-5 transition-transform duration-500 group-hover:translate-x-1"
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
      </Link>
    </div>
  );
}
