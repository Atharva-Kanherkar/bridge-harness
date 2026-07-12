import { useEffect, useRef, useState } from "react";
import { ArrowUp, ChevronDown, Plus } from "lucide-react";
import "../home.css";

// Pure UI mockup of the Bridge home / morning briefing. Typography-first,
// keyboard-driven (↑↓ + ↵), no data wiring — actions only dismiss so the
// design can be reviewed live.

const GmailLogo = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill="#EA4335" d="M24 5.457v13.909c0 .904-.732 1.636-1.636 1.636h-3.819V11.73L12 16.64l-6.545-4.91v9.273H1.636A1.636 1.636 0 0 1 0 19.366V5.457c0-2.023 2.309-3.178 3.927-1.964L5.455 4.64 12 9.548l6.545-4.91 1.528-1.145C21.69 2.28 24 3.434 24 5.457z"/>
  </svg>
);
const SlackLogo = ({ size = 13 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 122.8 122.8" aria-hidden>
    <path fill="#E01E5A" d="M25.8 77.6c0 7.1-5.8 12.9-12.9 12.9S0 84.7 0 77.6s5.8-12.9 12.9-12.9h12.9v12.9zm6.5 0c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9v32.3c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V77.6z"/>
    <path fill="#36C5F0" d="M45.2 25.8c-7.1 0-12.9-5.8-12.9-12.9S38.1 0 45.2 0s12.9 5.8 12.9 12.9v12.9H45.2zm0 6.5c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H12.9C5.8 58.1 0 52.3 0 45.2s5.8-12.9 12.9-12.9h32.3z"/>
    <path fill="#2EB67D" d="M97 45.2c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9-5.8 12.9-12.9 12.9H97V45.2zm-6.5 0c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V12.9C64.7 5.8 70.5 0 77.6 0s12.9 5.8 12.9 12.9v32.3z"/>
    <path fill="#ECB22E" d="M77.6 97c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9-12.9-5.8-12.9-12.9V97h12.9zm0-6.5c-7.1 0-12.9-5.8-12.9-12.9s5.8-12.9 12.9-12.9h32.3c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H77.6z"/>
  </svg>
);
const GitHubLogo = ({ size = 14, fill = "#e8e8ea" }: { size?: number; fill?: string }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill={fill} d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/>
  </svg>
);
const CalendarLogo = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <rect x="2.5" y="3.5" width="19" height="18" rx="3.5" fill="none" stroke="#8ab4f8" strokeWidth="1.7"/>
    <path d="M2.5 8.5h19" stroke="#8ab4f8" strokeWidth="1.7"/>
    <path d="M7 2v3.4M17 2v3.4" stroke="#8ab4f8" strokeWidth="1.7" strokeLinecap="round"/>
  </svg>
);
const NotionLogo = ({ size = 13 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <path fill="#c9c9ce" d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.373.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.886l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.933.653.933 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.448-1.632z"/>
  </svg>
);
const BridgeMark = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden>
    <path d="M3 17c0-5 4-9 9-9s9 4 9 9" stroke="#9a9aa2" strokeWidth="2" strokeLinecap="round"/>
    <path d="M3 17h18" stroke="#9a9aa2" strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

function greeting(now: Date) {
  const h = now.getHours();
  if (h < 5) return "Still up";
  if (h < 12) return "Good morning";
  if (h < 18) return "Good afternoon";
  return "Good evening";
}

type FeedItem = { logos: React.ReactNode[]; title: string; source: string; sub: string; urgent?: boolean };

const NEEDS_YOU: FeedItem[] = [
  { logos: [<GmailLogo key="g"/>], title: "Reply to Shreya — Rimo contract renewal is blocked on pricing", source: "Gmail", sub: "shreya@rimo.app · 11:48 PM", urgent: true },
  { logos: [<SlackLogo key="s"/>], title: "Parvati flagged a race in session cleanup, wants your take", source: "Slack", sub: "#eng-bridge · 7 replies · before standup", urgent: true },
  { logos: [<GitHubLogo key="gh"/>, <SlackLogo key="s"/>], title: "Review PR #21 — policy engine budget follow-up", source: "GitHub + Slack", sub: "+412 −96 · also raised in #eng-bridge, merged into one task" },
  { logos: [<GitHubLogo key="gh"/>], title: "Review PR #22 — sandbox writes behind capability tiers", source: "GitHub", sub: "+188 −40 · CI green" },
  { logos: [<GmailLogo key="g"/>], title: "Confirm Tuesday 4:30 PM call with Jared (YC follow-up)", source: "Gmail", sub: "jared@ycombinator.com · 6:02 AM" },
];

const FYI: Array<{ glyph: React.ReactNode; text: string; when: string }> = [
  { glyph: <GitHubLogo size={13} fill="#9a9aa2"/>, text: "Nightly CI flaked once on main, auto-retried, passed", when: "3:11 AM" },
  { glyph: <SlackLogo size={12}/>, text: "#design signed off the composer spacing — no action needed", when: "9:42 PM" },
  { glyph: <GmailLogo size={13}/>, text: "Stripe invoice paid, receipt archived", when: "8:15 PM" },
  { glyph: <CalendarLogo size={13}/>, text: "Clear afternoon — nothing between standup and the 4:30 call", when: "today" },
];

export function WelcomeScreen({ onDismiss }: { onDismiss: () => void }) {
  const [sel, setSel] = useState(0);
  const selRef = useRef(sel);
  selRef.current = sel;

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
      if (e.key === "ArrowDown") { e.preventDefault(); setSel(s => Math.min(s + 1, NEEDS_YOU.length - 1)); }
      if (e.key === "ArrowUp") { e.preventDefault(); setSel(s => Math.max(s - 1, 0)); }
      if (e.key === "Enter") { e.preventDefault(); onDismiss(); }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onDismiss]);

  const now = new Date();
  const dateLine = now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });

  return <div className="home">
    <header className="home-titlebar" data-tauri-drag-region>
      <div className="home-brand"><BridgeMark/> Bridge</div>
      <div className="home-sync">
        <span className="glyph"><GmailLogo size={12}/></span>
        <span className="glyph"><SlackLogo size={11}/></span>
        <span className="glyph"><GitHubLogo size={12} fill="#b9b9c0"/></span>
        <span className="glyph"><CalendarLogo size={12}/></span>
        <i/><small>synced 2m ago</small>
      </div>
      <button className="home-skip" onClick={onDismiss}>Workspaces <kbd>esc</kbd></button>
    </header>

    <div className="home-scroll">
      <div className="home-content">
        <div className="home-greeting">
          <h1>{greeting(now)}, Atharva.</h1>
          <p className="home-lede">
            While you were away: <b>two emails</b>, <b>two Slack threads</b> and <b>two pull requests</b> need
            you. I filtered fourteen more as noise and merged one duplicate.
          </p>
          <div className="home-meta">
            <span>{dateLine}</span>
            <span className="sep"/>
            <span>Standup in 1h 20m, then clear until 4:30</span>
          </div>
        </div>

        <div className="home-section" style={{ animationDelay: "80ms" }}>
          <label>Needs you</label><small>{NEEDS_YOU.length}</small>
        </div>
        <div className="feed-card" style={{ animationDelay: "110ms" }}>
          {NEEDS_YOU.map((item, index) => (
            <button
              key={item.title}
              className={`feed-row ${sel === index ? "selected" : ""}`}
              onMouseEnter={() => setSel(index)}
              onClick={onDismiss}
            >
              <span className="row-logos">{item.logos.map((logo, i) => <span className="logo-chip" key={i}>{logo}</span>)}</span>
              <span className="row-body">
                <span className="row-title">{item.title}</span>
                <span className="row-sub"><em>{item.source}</em> · {item.sub}</span>
              </span>
              {item.urgent && <span className="row-urgent"/>}
              <span className="row-action">Start chat <kbd>↵</kbd></span>
            </button>
          ))}
        </div>

        <div className="home-section" style={{ animationDelay: "360ms" }}>
          <label>Worth knowing</label><small>{FYI.length}</small>
        </div>
        <div className="feed-card quiet" style={{ animationDelay: "390ms" }}>
          {FYI.map(item => (
            <div className="fyi-row" key={item.text}>
              <span className="glyph">{item.glyph}</span>
              <span>{item.text}</span>
              <small>{item.when}</small>
            </div>
          ))}
        </div>

        <div className="home-endnote" style={{ animationDelay: "560ms" }}>
          <button><ChevronDown size={13}/> 14 filtered items</button>
          <button><Plus size={13}/> Connect Notion, Linear…</button>
        </div>
      </div>
    </div>

    <div className="home-composer">
      <div className="home-composer-inner">
        <div className="home-composer-shell">
          <span>Ask Bridge anything…</span>
          <button className="home-send" onClick={onDismiss}><ArrowUp size={15}/></button>
        </div>
        <div className="home-hints">
          <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
          <span><kbd>↵</kbd> start chat</span>
          <span><kbd>⌘K</kbd> search</span>
          <span><kbd>esc</kbd> workspaces</span>
        </div>
      </div>
    </div>
  </div>;
}
