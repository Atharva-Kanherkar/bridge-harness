# Bridge launch video — Test Contract

## Functional Behavior
- Deliver a rendered 1920×1080 H.264 MP4 launch film with an original instrumental soundtrack, plus a local HTML preview and editable Remotion source.
- Use the existing landing mockups and Graphite & Paper design tokens; style with Tailwind v4 and the existing single src/index.css stylesheet.
- Cover providers, switching, Mission Control, delegation, isolation, diffs, verification, history/compaction, memory, approved browser access, skills/plugins, Prompt Studio, automations, usage and terminal fleet.
- Claims must have repository evidence. Demonstration data must be identifiable as illustrative.
- Preserve existing unrelated working-tree changes.

## Unit Tests
N/A — presentation work. Verify timeline continuity and source coverage with a focused validation script.

## Integration / Functional Tests
- Typecheck the isolated video package.
- Render all scene midpoint stills and inspect representative frames.
- Render full movie; ffprobe confirms video dimensions, duration and audio track.

## Smoke Tests
- Build HTML preview with Vite and verify it serves successfully.

## E2E Tests
- Scrub preview through beginning, middle and end; ensure frames render and playback controls work.

## Manual Tests
- Inspect contact sheet for clipping, missing fonts, illegible copy and blank scenes.
- Verify output decodes with ffmpeg and audio has no clipping.
