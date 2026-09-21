import React, { useRef } from "react";
import { createRoot } from "react-dom/client";
import { Player, type PlayerRef } from "@remotion/player";
import { Film } from "./Film";
import { DURATION, FPS, timeline } from "./scenes";
function Preview() {
  const player = useRef<PlayerRef>(null);
  return (
    <main className="min-h-screen overflow-auto bg-background px-5 py-10 text-foreground sm:px-10">
      <div className="mx-auto max-w-7xl">
        <div className="mb-7 flex items-end justify-between">
          <div>
            <p className="font-mono text-xs uppercase tracking-widest text-muted-foreground">
              Bridge / Launch film
            </p>
            <h1 className="mt-2 font-display text-3xl">
              Your agents. One control room.
            </h1>
          </div>
          <span className="text-sm text-muted-foreground">
            {DURATION / FPS}s · 1080p · Original score
          </span>
        </div>
        <Player
          ref={player}
          component={Film}
          durationInFrames={DURATION}
          fps={FPS}
          compositionWidth={1920}
          compositionHeight={1080}
          controls
          clickToPlay
          className="aspect-video w-full! h-auto! overflow-hidden rounded-xl border border-border"
        />
        <p className="mt-6 text-sm text-muted-foreground">
          Choose a chapter to jump through the film. Product demonstrations use
          illustrative data.
        </p>
        <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
          {timeline.map((s) => (
            <button
              key={s.id}
              className="rounded-lg border border-border bg-card px-4 py-3 text-left text-sm hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
              onClick={() => {
                player.current?.seekTo(s.from + 15);
                player.current?.play();
              }}
            >
              <span className="mr-2 font-mono text-xs text-muted-foreground">
                {Math.floor(s.from / FPS / 60)}:
                {String((s.from / FPS) % 60).padStart(2, "0")}
              </span>
              {s.id === "intro"
                ? "Introduction"
                : s.id === "outro"
                  ? "Get Bridge"
                  : s.eyebrow.split("/ ")[1]}
            </button>
          ))}
        </div>
      </div>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<Preview />);
