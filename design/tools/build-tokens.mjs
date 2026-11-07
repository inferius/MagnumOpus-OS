// Generator tokenu: design/tokens.toml -> design/assets/tokens.css + tokens.js
// a kontrola kontrastu podle WCAG 2.2.
//
// Pouziti:  node design/tools/build-tokens.mjs          (zapise soubory)
//           node design/tools/build-tokens.mjs --check  (jen kontrola, nic nezapise)
// Navratovy kod 1, kdyz nektera dvojice nesplni minimum nebo vystup neni aktualni.

import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const checkOnly = process.argv.includes('--check');

// --- Parser podmnoziny TOML: [sekce.pod], klic = "text" | cislo, komentare #.
export function parseToml(text) {
  const out = {};
  let cur = out;
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.replace(/^\s+|\s+$/g, '');
    if (!line || line.startsWith('#')) return;
    const sec = line.match(/^\[([A-Za-z0-9_.]+)\]$/);
    if (sec) {
      cur = out;
      for (const part of sec[1].split('.')) cur = cur[part] ??= {};
      return;
    }
    // Retezec v uvozovkach (muze obsahovat #), nebo cislo; za hodnotou smi byt komentar.
    const kv = line.match(/^([A-Za-z0-9_]+)\s*=\s*("([^"]*)"|-?\d+(?:\.\d+)?)\s*(#.*)?$/);
    if (!kv) throw new Error(`tokens.toml:${i + 1}: neznamy radek nebo hodnota: ${raw}`);
    let v;
    if (kv[3] !== undefined) {
      v = kv[3];
    } else if (/^-?\d+(\.\d+)?$/.test(kv[2])) {
      v = Number(kv[2]);
    } else {
      throw new Error(`tokens.toml:${i + 1}: hodnota musi byt retezec nebo cislo`);
    }
    cur[kv[1]] = v;
  });
  return out;
}

// --- WCAG: relativni jas a kontrast.
function hexToRgb(hex) {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function luminance([r, g, b]) {
  const ch = (c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}
export function contrast(a, b) {
  const la = luminance(hexToRgb(a)), lb = luminance(hexToRgb(b));
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

const kebab = (s) => s.replace(/_/g, '-');
const shadowCss = (s) =>
  s === 'none'
    ? 'none'
    : s.split('|').map((layer) => {
        const p = layer.trim().split(/\s+/);
        const color = p.slice(4).join(' ');
        return `${p[0]}px ${p[1]}px ${p[2]}px ${p[3]}px ${color}`;
      }).join(', ');

const t = parseToml(readFileSync(join(root, 'tokens.toml'), 'utf8'));

// --- Kontrola: vsechny motivy maji stejne klice, dvojice splnuji minimum.
const themes = ['light', 'dark', 'contrast'];
const keys = Object.keys(t.color.light);
let failed = 0;
for (const th of themes) {
  for (const k of keys) {
    if (!(k in t.color[th])) { console.error(`chybi color.${th}.${k}`); failed++; }
  }
}
const report = [];
for (const th of themes) {
  for (const [name, spec] of Object.entries(t.contrast)) {
    const [fg, bg, minStr] = spec.split(/\s+/);
    let min = Number(minStr);
    if (th === 'contrast') min = min >= 4.5 ? 7 : 4.5;
    const a = t.color[th][fg], b = t.color[th][bg];
    if (!hexToRgb(a) || !hexToRgb(b)) { console.error(`${th}.${name}: neni #rrggbb`); failed++; continue; }
    // Vysoky kontrast: "soft" pozadi je cerne, tlacitko s vyplni je zluta - dvojice plati taky.
    const r = contrast(a, b);
    const ok = r >= min;
    if (!ok) failed++;
    report.push({ theme: th, name, fg, bg, ratio: Math.round(r * 100) / 100, min, ok });
  }
}
for (const r of report.filter((x) => !x.ok)) {
  console.error(`KONTRAST ${r.theme}.${r.name}: ${r.fg} na ${r.bg} = ${r.ratio} < ${r.min}`);
}

// --- CSS.
function colorBlock(th) {
  return Object.entries(t.color[th]).map(([k, v]) => `  --c-${kebab(k)}: ${v};`).join('\n');
}
function shadowBlock(th) {
  return Object.entries(t.shadow[th]).map(([k, v]) => `  --sh-${kebab(k)}: ${shadowCss(v)};`).join('\n');
}
const lines = [];
lines.push('/* VYGENEROVANO z design/tokens.toml (node design/tools/build-tokens.mjs). Neupravovat rucne. */');
lines.push(':root {');
lines.push(`  --font-ui: "${t.font.ui}", "${t.font.ui_fallback}", system-ui, sans-serif;`);
lines.push(`  --font-mono: "${t.font.mono}", "${t.font.mono_fallback}", ui-monospace, monospace;`);
lines.push('  --text-scale: 1;');
for (const [name, ty] of Object.entries(t.type)) {
  if (name === 'scale') continue;
  lines.push(`  --t-${kebab(name)}-size: calc(${ty.size}px * var(--text-scale));`);
  lines.push(`  --t-${kebab(name)}-line: calc(${ty.line}px * var(--text-scale));`);
  lines.push(`  --t-${kebab(name)}-weight: ${ty.weight};`);
}
for (const [k, v] of Object.entries(t.space)) lines.push(`  --${k}: ${v}px;`);
for (const [k, v] of Object.entries(t.size)) lines.push(`  --size-${kebab(k)}: ${v}${k === 'icon_stroke' ? '' : 'px'};`);
for (const [k, v] of Object.entries(t.radius)) lines.push(`  --r-${kebab(k)}: ${v}px;`);
for (const [k, v] of Object.entries(t.motion)) {
  if (k.startsWith('ease_')) lines.push(`  --${kebab(k)}: cubic-bezier(${v.split(/\s+/).join(', ')});`);
  else lines.push(`  --m-${kebab(k)}: ${v}ms;`);
}
for (const [k, v] of Object.entries(t.layer)) lines.push(`  --z-${kebab(k)}: ${v};`);
lines.push(colorBlock('light'));
lines.push(shadowBlock('light'));
lines.push('  color-scheme: light;');
lines.push('}');
lines.push('/* Bez skriptu: podle systemu. Se skriptem nastavi data-theme mo.js (podle denni doby). */');
lines.push('@media (prefers-color-scheme: dark) {');
lines.push('  :root:not([data-theme="light"]) {');
lines.push(colorBlock('dark').replace(/^/gm, '  '));
lines.push(shadowBlock('dark').replace(/^/gm, '  '));
lines.push('    color-scheme: dark;');
lines.push('  }');
lines.push('}');
lines.push(':root[data-theme="dark"] {');
lines.push(colorBlock('dark'));
lines.push(shadowBlock('dark'));
lines.push('  color-scheme: dark;');
lines.push('}');
lines.push(':root[data-contrast="more"] {');
lines.push(colorBlock('contrast'));
lines.push(shadowBlock('contrast'));
lines.push('  color-scheme: dark;');
lines.push('}');
lines.push('');
const css = lines.join('\n');

const js =
  '// VYGENEROVANO z design/tokens.toml (node design/tools/build-tokens.mjs). Neupravovat rucne.\n' +
  `window.MO_TOKENS = ${JSON.stringify(t, null, 1)};\n` +
  `window.MO_CONTRAST = ${JSON.stringify(report)};\n`;

const cssPath = join(root, 'assets', 'tokens.css');
const jsPath = join(root, 'assets', 'tokens.js');
if (checkOnly) {
  const same = (p, s) => existsSync(p) && readFileSync(p, 'utf8') === s;
  if (!same(cssPath, css) || !same(jsPath, js)) {
    console.error('assets/tokens.css nebo tokens.js neodpovida tokens.toml - spust build-tokens.mjs');
    failed++;
  }
} else {
  writeFileSync(cssPath, css);
  writeFileSync(jsPath, js);
}
const passed = report.filter((r) => r.ok).length;
console.log(`kontrast: ${passed}/${report.length} dvojic v poradku`);
process.exit(failed ? 1 : 0);
