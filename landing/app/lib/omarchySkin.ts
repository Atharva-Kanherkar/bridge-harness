// The landing page's Omarchy mode: the whole site, the hero demo included,
// retinted through the same token table the desktop app uses on Omarchy.
//
// Tokens are computed at build time, and an inline script applies the stored
// choice before first paint so a reload never flashes Graphite. The script
// also leaves a tiny controller on `window` that the toggle and the theme
// strip call; React never owns these attributes, so hydration never fights.

import { DEFAULT_OMARCHY_THEME, OMARCHY_THEMES } from "../content/omarchyThemes";
import { OMARCHY_TOKEN_NAMES, omarchyTokens } from "./omarchyTheme";

export const SKIN_STORAGE_KEY = "bridge-site-skin";
export const THEME_STORAGE_KEY = "bridge-site-omarchy-theme";
/** Fired on `window` whenever the mode or the theme changes. */
export const OMARCHY_CHANGE_EVENT = "bridge-omarchy-change";

export type OmarchyController = {
  /** The theme showing, or null when the site is in its Graphite skin. */
  current: () => string | null;
  /** Turn Omarchy mode on with a theme, or off with null. Persists. */
  apply: (theme: string | null) => void;
};

declare global {
  interface Window {
    __bridgeOmarchy?: OmarchyController;
  }
}

type ThemeData = { mode: string; tokens: Record<string, string> };

export function themeData(): Record<string, ThemeData> {
  return Object.fromEntries(OMARCHY_THEMES.map(theme => [theme.name, { mode: theme.mode, tokens: omarchyTokens(theme) }]));
}

/**
 * The pre-paint script. Plain ES5 so it runs before any bundle, and
 * self-contained so it can be serialised into the document head.
 */
export function prepaintScript(data: Record<string, ThemeData> = themeData()): string {
  const config = {
    themes: data,
    names: OMARCHY_TOKEN_NAMES,
    fallback: DEFAULT_OMARCHY_THEME,
    skinKey: SKIN_STORAGE_KEY,
    themeKey: THEME_STORAGE_KEY,
    event: OMARCHY_CHANGE_EVENT,
  };
  return `(function(c){
var root=document.documentElement,shown=null;
function read(k){try{return localStorage.getItem(k)}catch(e){return null}}
function write(k,v){try{localStorage.setItem(k,v)}catch(e){}}
function paint(name){
  var t=name&&c.themes[name];
  for(var i=0;i<c.names.length;i++)root.style.removeProperty(c.names[i]);
  if(!t){shown=null;delete root.dataset.skin;delete root.dataset.omarchyTheme;root.style.removeProperty("color-scheme");return}
  for(var k in t.tokens)root.style.setProperty(k,t.tokens[k]);
  root.style.setProperty("color-scheme",t.mode);
  root.dataset.skin="omarchy";root.dataset.omarchyTheme=name;shown=name;
}
function apply(name){
  if(name&&!c.themes[name])name=c.fallback;
  write(c.skinKey,name?"omarchy":"graphite");
  if(name)write(c.themeKey,name);
  paint(name);
  try{window.dispatchEvent(new Event(c.event))}catch(e){}
}
var q=null;try{q=new URLSearchParams(location.search).get("omarchy")}catch(e){}
if(q==="off")apply(null);
else if(q!==null)apply(c.themes[q]?q:(c.themes[read(c.themeKey)]?read(c.themeKey):c.fallback));
else if(read(c.skinKey)==="omarchy")paint(c.themes[read(c.themeKey)]?read(c.themeKey):c.fallback);
window.__bridgeOmarchy={current:function(){return shown},apply:apply};
})(${JSON.stringify(config)});`;
}
