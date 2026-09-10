# Bridge landing site plan

Modeled on onorca.dev (Orca by Stably: Next.js, Motion, dark chrome, real-DOM mockups, 64.7k stars). Same skeleton and rhythm; Bridge's own claims, fonts, and tokens.

## Orca's page, section by section, and Bridge's answer

| # | Orca | Bridge | When |
|---|---|---|---|
| 1 | Sticky nav: logo, Docs, Changelog, Enterprise, Discord, X, GitHub star count, Download | Bridge, Docs, Changelog, GitHub, Download. No star count while the repo is private. | Done |
| 2 | Hero: "Backed by Y Combinator" badge, "Ship 100x with the agent IDE", one concrete subhead, Download for Mac + "Also for Intel · Windows · Linux", View on GitHub | Eyebrow "macOS 12 or later · Apple Silicon · v0.5.5", "Delegate the coding. Keep the judgment.", subhead, Download for Mac, View on GitHub | Done |
| 3 | Tabbed real-DOM app mockup, 5 tabs | Five real captures of the app in content/hero.ts driving HeroFrame | Done |
| 4 | "Used by engineers at" logo strip | Skip until real logos exist | Never fabricate |
| 5 | "Your dev loop, agentified." vertical tabs (Workspaces, Orchestration, Browser, Terminal, Tasks, Editor, Notes, Ship with AI), each with a mini mock | "Your dev loop, supervised." Six vertical tabs: Workspaces, Delegation, Conversation, Work board, Terminal and browser, Ship | Done |
| 6 | "Bring your own agent / subscription", 27 agent logos | "Bring your own harness": three cards for Codex, Claude Code, OpenCode with harness tints | Done |
| 7 | Mobile companion section | Skip, no companion app | Never |
| 8 | "Agent-first, end to end." 13-tile bento, poster images, "Click to inspect" | "Supervised, end to end." 13 text-first tiles; inspect dialogs once real captures exist | Tiles done, Phase 3 inspect |
| 9 | Eight testimonials from X | Skipped. Replaced with a "Three rules" principles section | Done |
| 10 | Comparison table "Built for agents, not retrofitted" | "Built for supervision, not retrofitted": eight rows, Bridge against the category | Done |
| 11 | FAQ, ten collapsed questions | FAQ, ten questions, native details elements | Done |
| 12 | "Get Orca" CTA band | "Get Bridge" CTA band | Done |
| 13 | Footer: Product, Community, Company, copyright, "Backed by Y Combinator. Built in San Francisco." | Footer: brand, Product, Project, version line | Done |
| 14 | /download page: four desktop builds, brew command, mobile links | /download: Apple Silicon DMG, checksum command, requirements, Node 18+ note for Claude | Done |
| 15 | /changelog: date, version link, headline, bullets, "Read more", per-version pages | /changelog from release notes, one page, "All releases on GitHub" | Done |

## Phase 1: finish the one-page skeleton (done)

1. Sticky nav with backdrop blur and a hairline that appears on scroll. Changelog points at GitHub releases until the route exists.
2. "Bring your own harness" section: three cards. Codex over its app-server JSON-RPC protocol, Claude Code through the Agent SDK sidecar, OpenCode through its headless server. Harness tint dot per card from the harness tokens. Closing line: a missing CLI shows its adapter as unavailable instead of blocking startup.
3. Replace the six-card feature grid with a 13-tile bento, "Supervised, end to end." Tiles: Policy engine; Isolated worktrees; Session forest; Typed worker results; Cross-harness verification; Checkpoints and resume; Usage and budgets; Daemon and CLI (bridge exec --json); Generated JSON-RPC protocol; Authenticated browser bridge; Managed runtimes; Work board and terminal. Two tiles span two columns, like Orca.
4. "Get Bridge" CTA band above a three-column footer (Product: Download, Changelog, Docs. Community: GitHub. Project: License, Privacy when they exist).
5. Scroll-in motion: IntersectionObserver adds the existing animate-fade-up utility. No Motion dependency yet.

Built as SiteHeader, HarnessSection, FeatureGrid, CallToAction, SiteFooter, and a Reveal wrapper. Reveal arms itself only after mount, so the prerendered HTML is visible without JavaScript and reduced-motion viewers skip the animation entirely. The two wide tiles sit at positions one and three so the three-column grid fills five rows with no gaps.

## Phase 2: depth sections (done)

1. "Your dev loop, supervised." Vertical tab list on the left, heading plus two sentences plus a mini mock on the right, built from the entry primitives in MockEntry. Data in content/loop.ts. Same keyboard model as the hero capability strip in HeroFrame.
2. Comparison table, eight rows. Built with two verdict columns rather than the planned three, because naming two rival categories separately would have meant claims about products that change weekly. A caption says the right column generalizes across a category.
3. "Three rules" section in the testimonial slot: the three hierarchies stay separate; learning and routing can rank but never grant; history is appended, never rewritten. One sentence each, lifted from docs/.
4. FAQ: What is Bridge? How is it different from running claude or codex in a terminal? Which agents does it support? Do I need API keys? Is it macOS only? Is it open source? What is a session forest? What does the policy engine gate? Can CI use it? Where does my data live?

The mock card renderer moved to its own module so the loop panels and the hero mockup share one implementation. The scroll-reveal wrapper added in Phase 1 was removed: it hid content until a callback fired, and a page that can render nothing is worse than a page without an entrance animation. A pure-CSS scroll timeline guarded by a support query would give the same effect without that failure mode, and is the way to add it back.

## Phase 3: routes and media (routes done, media outstanding)

The blog and docs routes were added beyond the original table: `/blog` with three posts drawn from the design references, and `/docs` rendering twelve of the repository's own markdown files rather than linking to GitHub.

1. /download: Apple Silicon DMG button to releases/latest, sha256 line, requirements (macOS 12 or later, Node 18+ for Claude), the Gatekeeper note from the README.
2. /changelog: six releases from 0.5.0 to 0.5.5, each with date, headline, bullets, and a link to its GitHub notes. Written as a typed data file rather than MDX, which avoids adding an MDX toolchain for prose that is already short. Copied from release notes at authoring time; no GitHub API because the repo is private.
3. Bento tiles open a dialog with a larger mock, then a short recording once real captures exist.
4. Open Graph image and metadataBase once the domain is chosen.
5. Done: scroll-in reveals and the capability illustrations run on CSS scroll timelines (`reveal`, `ill-part` utilities in globals.css) behind an @supports guard, so unsupported browsers and reduced-motion viewers see the finished state.

## Honesty gates, owner decisions

- No testimonials, customer logos, or star counts until they are real.
- No "open source" wording until a license is published. The README says all rights reserved.
- Three harnesses, macOS on Apple Silicon, no mobile app, no SSH worktrees. Never imply otherwise.
- Domain for metadataBase and the Docs link target (docs-site is not deployed yet).

## Carried over from Orca

- Orca uses real-DOM mockups. Bridge went the other way after the DOM copy drifted from the product: the hero shows real captures from the app's preview mode, recaptured with each redesign. Smaller mocks in the loop section stay as DOM.
- Dark achromatic chrome; color appears only inside mockups and only for meaning.
- One headline step, hairline dividers, max-w-6xl, generous vertical rhythm.
- Tabs, tiles, and FAQ items are real controls with roles.

## Stays Bridge

- Instrument Serif display (italic for the emphasized clause), Geist body, Geist Mono for labels and eyebrows.
- Tokens mirror the Graphite palette in src/index.css. No new palette.
- Tailwind v4 utilities only, CSS-first config in app/globals.css.
