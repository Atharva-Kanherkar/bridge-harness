import React from "react";
import { Composition, registerRoot } from "remotion";
import { Film } from "./Film";
import { DURATION, FPS } from "./scenes";
registerRoot(() => (
  <Composition
    id="BridgeLaunch"
    component={Film}
    durationInFrames={DURATION}
    fps={FPS}
    width={1920}
    height={1080}
  />
));
