// Every `var(--token)` a stylesheet uses must actually be defined -- and be
// defined in the *base* :root, not only inside the dark block.
//
// This exists because of a real bug: MusicView.css invented its own names
// with light fallbacks (`var(--surface, #fff)`), and since no --surface
// token exists anywhere, every button and the whole transport bar rendered
// pure white on a #1e1e1e window. A fallback is what makes this class of
// mistake invisible in one theme and glaring in the other.
//
// Tokens set inline from JS (`style={{ "--kind": ... }}`) are allowed by
// listing them below; a neutral fallback (a grey that works either way) is
// allowed too, since that is a deliberate pattern in App.css.
//
//   node scripts/check-theme-tokens.mjs

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const SET_FROM_JS = new Set(["--kind", "--list-pane-width", "--audio-progress"]);

const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");

const files = [];
(function walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (name.endsWith(".css")) files.push(path);
  }
})("src");

const app = stripComments(readFileSync("src/App.css", "utf8"));
const defined = new Set([...app.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
const base = new Set(
  [...app.split("@media (prefers-color-scheme: dark)")[0].matchAll(/(--[a-z0-9-]+)\s*:/g)]
    .map((m) => m[1]),
);

let bad = 0;
for (const file of files) {
  const css = stripComments(readFileSync(file, "utf8"));
  const local = new Set([...css.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
  // A `var(--x, something)` with a fallback is fine; a bare `var(--x)` is
  // not, unless the token exists.
  for (const match of css.matchAll(/var\((--[a-z0-9-]+)\s*(,)?/g)) {
    const [, token, hasFallback] = match;
    if (hasFallback || local.has(token) || SET_FROM_JS.has(token)) continue;
    if (!defined.has(token)) {
      console.error(`${file}: var(${token}) is not defined anywhere`);
      bad++;
    } else if (!base.has(token)) {
      console.error(`${file}: var(${token}) is only defined for dark mode`);
      bad++;
    }
  }
}

console.log(bad === 0 ? `theme tokens: ${files.length} stylesheets, all resolve` : `${bad} problem(s)`);
process.exit(bad === 0 ? 0 : 1);
