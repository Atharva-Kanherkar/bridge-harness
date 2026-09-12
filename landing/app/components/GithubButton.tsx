import { repoUrl } from "../content/site";

/*
 * Secondary action next to the download buttons: same edge-glow language as
 * ActionButton, but icon-led and dimmer at rest so it reads as "also
 * available here" rather than the primary call to action.
 */
export default function GithubButton() {
  return (
    <span className="group relative inline-block">
      <a
        href={repoUrl}
        target="_blank"
        rel="noopener noreferrer"
        aria-label="View Bridge on GitHub"
        className="relative inline-flex items-center justify-center rounded-xl p-px leading-6 text-foreground opacity-80 shadow-lg shadow-black/60 transition-transform duration-300 ease-in-out hover:scale-[1.03] hover:opacity-100 active:scale-95"
      >
        <span
          aria-hidden="true"
          className="absolute -inset-1 rounded-2xl bg-linear-to-r from-foreground/35 via-foreground/80 to-foreground/35 opacity-0 blur-lg transition-opacity duration-500 group-hover:opacity-25"
        />
        <span className="absolute inset-0 rounded-xl bg-linear-to-r from-foreground/35 via-foreground/80 to-foreground/35 opacity-40 transition-opacity duration-500 group-hover:opacity-100" />
        <span className="relative z-10 flex size-11 items-center justify-center rounded-xl bg-background sm:size-12">
          <svg viewBox="0 0 24 24" aria-hidden="true" className="size-5 fill-current">
            <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.09 3.29 9.4 7.86 10.92.57.1.78-.25.78-.55 0-.27-.01-1.16-.02-2.11-3.2.7-3.88-1.36-3.88-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.72.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.55-.29-5.23-1.28-5.23-5.68 0-1.25.45-2.28 1.18-3.08-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.18a10.9 10.9 0 0 1 2.87-.39c.97.01 1.95.13 2.87.39 2.19-1.49 3.15-1.18 3.15-1.18.62 1.58.23 2.75.11 3.04.73.8 1.18 1.83 1.18 3.08 0 4.41-2.69 5.38-5.25 5.67.41.36.78 1.06.78 2.14 0 1.55-.01 2.79-.01 3.17 0 .3.2.66.79.55A10.52 10.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z" />
          </svg>
        </span>
      </a>
    </span>
  );
}
