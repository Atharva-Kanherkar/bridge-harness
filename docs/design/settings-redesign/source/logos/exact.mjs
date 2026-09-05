// Exact marks built from the reference images Atharva supplied.
// Cursor: regular pointy-top hexagon, faceted. Tones from the reference.
const c = [12, 12], r = 8.6;
const v = a => [c[0] + r * Math.sin(a * Math.PI / 180), c[1] - r * Math.cos(a * Math.PI / 180)].map(n => n.toFixed(2)).join(" ");
const top = v(0), ur = v(60), lr = v(120), bot = v(180), ll = v(240), ul = v(300), ctr = "12 12";
export const CURSOR = (light = "#ededed", mid1 = "#8f8f8f", mid2 = "#6f6f6f", dark = "#4a4a4a", wedge = "#cfcfcf") => `
<polygon points="${top} ${ur} ${ul}" fill="${mid2}"></polygon>
<polygon points="${ul} ${ur} ${ctr}" fill="${light}"></polygon>
<polygon points="${ul} ${ctr} ${bot} ${ll}" fill="${mid1}"></polygon>
<polygon points="${ur} ${lr} ${bot}" fill="${dark}"></polygon>
<polygon points="${ctr} ${ur} ${bot}" fill="${wedge}"></polygon>
<polygon points="${top} ${ur} ${lr} ${bot} ${ll} ${ul}" fill="none" stroke="${mid1}" stroke-width="0.6" stroke-linejoin="round"></polygon>`;
// OpenCode: 4x5 module frame, 1-module border, inner 2x3: top module open, lower two modules muted.
export const OPENCODE = (frame, block) => `
<path fill-rule="evenodd" d="M2.4 0h19.2v24H2.4zM7.2 4.8v14.4h9.6V4.8z" fill="${frame}"></path>
<rect x="7.2" y="9.6" width="9.6" height="9.6" fill="${block}"></rect>`;
