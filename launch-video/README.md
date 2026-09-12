# Bridge launch film

A 102-second, 1920×1080, 30 fps launch film built with Remotion and React/HTML. The look comes from the existing landing page, the app's Graphite & Paper tokens, provider marks, transcript mockups and repository product captures.

## Watch

- Final movie: `out/bridge-launch.mp4`
- File-based video player: `out/watch.html` (open directly after rendering)
- Production HTML preview: `http://127.0.0.1:1473` while `npm run preview` is running
- Live HTML preview: `http://127.0.0.1:1472` while `npm run dev` is running
- Static preview build: `dist/index.html` (serve `dist/` over HTTP)
- Scene stills: `out/01-intro.png` through `out/16-outro.png`

## Reproduce

From the repository root, install the app dependencies first (`bun install`) so its shared font assets are available. Then:

```sh
cd launch-video
npm ci
python3 -m pip install numpy
python3 prepare.py
node verify.mjs
npm run build
npm run preview # optional; use another terminal for the remaining commands
npm run test:preview
npm run stills
npm run render
# Optional encoded-media validation (requires ffmpeg, ffprobe, Pillow and NumPy):
python3 validate-media.py
npm run dev
```

For `validate-media.py`, install Pillow as well (`python3 -m pip install Pillow`).

Python 3 with NumPy generates the original 110 BPM stereo instrumental; there are no downloaded music samples or paid generation calls. Remotion downloads a local headless Chrome on first render. `ffmpeg` and `ffprobe` are used for output verification, not required to edit the composition.

`npm run studio` opens the Remotion timeline. Edit chapter copy, durations and evidence in `src/scenes.ts`; edit animated HTML in `src/Film.tsx`. All motion is derived from the current frame, and fonts load before capture. The renderer uses software ANGLE (`swangle`) to avoid missing text layers observed with the default macOS headless GPU path. Styling is Tailwind v4 compiled from the repository's single `src/index.css`, which explicitly includes the video and reused landing component sources.

## Coverage and evidence

Every chapter records its repository evidence in `src/scenes.ts`. The film covers agent switching, Mission Control, role delegation, isolated worktrees, diff review, approvals, verification, history, rewind/forks, context checkpoints, memory, approved browser access, skills/plugins, GitHub, Prompt Studio, automations, usage and the terminal fleet.

The UI sequences are illustrative workflows; verification counts and task details are demonstration data. Screenshots come from `docs/media/`. There is an instrumental score and on-screen copy, but no voiceover. The closing GitHub address is the repository's actual download destination.

## Technical references

- [Remotion Tailwind v4 integration](https://www.remotion.dev/docs/tailwind-v4/enable-tailwind)
- [Remotion media renderer](https://www.remotion.dev/docs/renderer/render-media)

Generated media, copied images and builds are ignored by Git. `prepare.py` rebuilds the audio and copies the repository artwork. No upload, publication or release is performed.
