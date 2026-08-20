// Real connector marks for suggested-work rows: inline SVG, no external asset
// fetches, every path bundled here. The glyph is never load-bearing — the source
// is always also named in text beside it (workTasks.ts `sourceLabel`), so a row
// with no recognisable mark reads exactly the same.

export type ConnectorLogo = (props: { size?: number }) => React.ReactNode;

const SlackLogo: ConnectorLogo = ({ size = 13 }) => (
  <svg width={size} height={size} viewBox="0 0 122.8 122.8" aria-hidden>
    <path fill="currentColor" d="M25.8 77.6c0 7.1-5.8 12.9-12.9 12.9S0 84.7 0 77.6s5.8-12.9 12.9-12.9h12.9v12.9zm6.5 0c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9v32.3c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V77.6z"/>
    <path fill="currentColor" d="M45.2 25.8c-7.1 0-12.9-5.8-12.9-12.9S38.1 0 45.2 0s12.9 5.8 12.9 12.9v12.9H45.2zm0 6.5c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H12.9C5.8 58.1 0 52.3 0 45.2s5.8-12.9 12.9-12.9h32.3z"/>
    <path fill="currentColor" d="M97 45.2c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9-5.8 12.9-12.9 12.9H97V45.2zm-6.5 0c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V12.9C64.7 5.8 70.5 0 77.6 0s12.9 5.8 12.9 12.9v32.3z"/>
    <path fill="currentColor" d="M77.6 97c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9-12.9-5.8-12.9-12.9V97h12.9zm0-6.5c-7.1 0-12.9-5.8-12.9-12.9s5.8-12.9 12.9-12.9h32.3c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H77.6z"/>
  </svg>
);

const GmailLogo: ConnectorLogo = ({ size = 14 }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill="currentColor" d="M24 5.457v13.909c0 .904-.732 1.636-1.636 1.636h-3.819V11.73L12 16.64l-6.545-4.91v9.273H1.636A1.636 1.636 0 0 1 0 19.366V5.457c0-2.023 2.309-3.178 3.927-1.964L5.455 4.64 12 9.548l6.545-4.91 1.528-1.145C21.69 2.28 24 3.434 24 5.457z"/>
  </svg>
);

const GitHubLogo: ConnectorLogo = ({ size = 14 }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill="currentColor" d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/>
  </svg>
);

const LinearLogo: ConnectorLogo = ({ size = 13 }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path
      fill="currentColor"
      d="M2.886 14.407a.226.226 0 0 1 .062-.213L14.194 2.948a.226.226 0 0 1 .213-.062 9.7 9.7 0 0 1 3.516 1.706L4.592 17.923a9.7 9.7 0 0 1-1.706-3.516ZM2.03 11.038a.23.23 0 0 0 .064.196l10.672 10.672c.052.052.126.076.196.064a9.9 9.9 0 0 0 1.936-.463L2.492 9.101a9.9 9.9 0 0 0-.463 1.937Zm.677-3.86 14.115 14.116a9.8 9.8 0 0 0 1.442-.968L3.674 5.735a9.8 9.8 0 0 0-.968 1.442Zm2.317-3.14L19.962 18.976A9.97 9.97 0 0 0 22 12C22 6.477 17.523 2 12 2a9.97 9.97 0 0 0-6.976 2.038Z"
    />
  </svg>
);

const NotionLogo: ConnectorLogo = ({ size = 13 }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill="currentColor" d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.373.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.886l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.933.653.933 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.448-1.632z"/>
  </svg>
);

/** Marks by connector family — the same five families evidence resolution knows. */
export const CONNECTOR_LOGOS: Record<string, ConnectorLogo> = {
  slack: SlackLogo,
  gmail: GmailLogo,
  github: GitHubLogo,
  linear: LinearLogo,
  notion: NotionLogo,
};

/** The mark for a task's `sourceKind` ("slack.message" → Slack), or none. */
export function logoForSourceKind(sourceKind: string): ConnectorLogo | undefined {
  return CONNECTOR_LOGOS[sourceKind.split(".")[0] ?? ""];
}
