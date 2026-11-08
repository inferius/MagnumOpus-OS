// Generator sady ikon ve smeru F (pastelovy prechod).
//
// Pouziti: node design/tools/build-icons.mjs <slozka-se-svg-24>
//   <slozka> = svg/24 ze zdrojove sady (162 linkovych ikon, tah 1,75).
// Vystup: blok mezi znackami ICONS-START / ICONS-END v design/assets/mo.js:
//   ICONS[jmeno] = { c: kategorie, f: vypln (SVG), l: kresba (SVG) }.
// Kategorie urcuje barvu (tokeny icon_<kat>_bg a icon_<kat>); "ui" = bez vyplne,
// barva textu (sipky, zavrit, plus...). ID ikon (ic-<jmeno>) se nemeni.

import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const src = process.argv[2];
if (!src) { console.error('pouziti: node build-icons.mjs <slozka-svg-24>'); process.exit(2); }

// ---- Kategorie (barva). Co neni vyjmenovane, je "system".
const CAT = {
  files: 'folder folder-plus file file-plus file-text file-audio file-video disk trash archive backup database download upload save package install paste copy cut eject',
  network: 'wifi wifi-low wifi-off globe network ethernet bluetooth bluetooth-off cloud vpn hotspot airplane link share send bell bell-off search filter sort tasks list log activity info help external language captions',
  devices: 'volume volume-low volume-off battery battery-charge battery-empty battery-half battery-low camera camera-off mic mic-off headphones music video image printer keyboard mouse touchpad usb plug monitor play pause stop next previous screen-share sun moon gauge hearing desktop',
  time: 'history clock calendar branch merge diff conflict rollback undo redo update refresh snapshot restore preview restart',
  agent: 'agent',
  ui: 'close min max plus minus check left right up down arrow-up more menu move edit rename select-all zoom-in zoom-out fullscreen exit-fullscreen pin eye eye-off alert error star sparkdot'
};
const catOf = {};
for (const [c, list] of Object.entries(CAT)) for (const n of list.split(/\s+/)) catOf[n] = c;

// ---- Prekreslene slabsi ikony (kresba a vypln rucne).
const REDRAW = {
  usb: { l: '<rect x="7" y="9.5" width="10" height="11.5" rx="3"/><path d="M8.5 9.5V4.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5"/><path d="M11 6.5h.01M13 6.5h.01"/><path d="M10 14h4"/>',
    f: '<rect x="7" y="9.5" width="10" height="11.5" rx="3"/>' },
  diff: { l: '<rect x="3" y="4" width="8" height="16" rx="2.5"/><rect x="13" y="4" width="8" height="16" rx="2.5"/><path d="M5.5 9h3M5.5 13h3M15.5 9h3M15.5 13h3M17 11.5v3"/>',
    f: '<rect x="13" y="4" width="8" height="16" rx="2.5"/>' },
  conflict: { l: '<circle cx="6" cy="5" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 7c0 3.5 3 4.5 5 6M18 7c0 3.5-3 4.5-5 6"/><path d="M12.6 13.2l-2 3.4h3l-2 3.6"/>',
    f: '<circle cx="6" cy="5" r="2"/><circle cx="18" cy="5" r="2"/>' },
  // Mekci tvary ze smeru F (vetsi zaobleni, oblejsi rohy).
  folder: { l: '<path d="M3 8a3 3 0 0 1 3-3h3.6a2 2 0 0 1 1.4.6L12.5 7H18a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z"/>',
    f: '<path d="M3 8a3 3 0 0 1 3-3h3.6a2 2 0 0 1 1.4.6L12.5 7H18a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z"/>' },
  file: { l: '<path d="M8 3h5.6a2 2 0 0 1 1.4.6l3.4 3.4a2 2 0 0 1 .6 1.4V18a3 3 0 0 1-3 3H8a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z"/><path d="M14 3.5V7a1.5 1.5 0 0 0 1.5 1.5H19"/>',
    f: '<path d="M8 3h5.6a2 2 0 0 1 1.4.6l3.4 3.4a2 2 0 0 1 .6 1.4V18a3 3 0 0 1-3 3H8a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z"/>' },
  terminal: { l: '<rect x="3" y="4" width="18" height="16" rx="4.5"/><path d="M7.5 9.5l2.5 2.5-2.5 2.5M12.5 15H16.5"/>', f: '<rect x="3" y="4" width="18" height="16" rx="4.5"/>' },
  bell: { l: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.2 1.6a.9.9 0 0 1-.7 1.4h-13a.9.9 0 0 1-.7-1.4z"/><path d="M10 21a2 2 0 0 0 4 0"/>',
    f: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.2 1.6a.9.9 0 0 1-.7 1.4h-13a.9.9 0 0 1-.7-1.4z"/>' },
  trash: { l: '<path d="M4.5 7h15"/><path d="M9.5 7V5.5A1.5 1.5 0 0 1 11 4h2a1.5 1.5 0 0 1 1.5 1.5V7"/><path d="M6.5 7l.8 11.2A3 3 0 0 0 10.3 21h3.4a3 3 0 0 0 3-2.8L17.5 7"/>',
    f: '<path d="M6.5 7l.8 11.2A3 3 0 0 0 10.3 21h3.4a3 3 0 0 0 3-2.8L17.5 7z"/>' },
  volume: { l: '<path d="M4 10.5a1 1 0 0 1 1-1h2.5l3.6-3.2a.6.6 0 0 1 .9.4v10.6a.6.6 0 0 1-.9.4l-3.6-3.2H5a1 1 0 0 1-1-1z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
    f: '<path d="M4 10.5a1 1 0 0 1 1-1h2.5l3.6-3.2a.6.6 0 0 1 .9.4v10.6a.6.6 0 0 1-.9.4l-3.6-3.2H5a1 1 0 0 1-1-1z"/>' },
  lock: { l: '<rect x="5" y="10.5" width="14" height="10" rx="3.5"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/><circle cx="12" cy="15.5" r=".9" fill="currentColor"/>', f: '<rect x="5" y="10.5" width="14" height="10" rx="3.5"/>' },
  shieldcheck: { l: '<path d="M12 3.2l6.2 2.4a2 2 0 0 1 1.3 1.9v4c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5v-4a2 2 0 0 1 1.3-1.9z"/><path d="M8.7 12.2l2.3 2.3 4.3-4.6"/>',
    f: '<path d="M12 3.2l6.2 2.4a2 2 0 0 1 1.3 1.9v4c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5v-4a2 2 0 0 1 1.3-1.9z"/>' },
  disk: { l: '<rect x="3" y="5" width="18" height="14" rx="4"/><path d="M3 13h18"/><circle cx="17" cy="16" r=".9" fill="currentColor"/>', f: '<path d="M3 13h18v2a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4z"/>' },
  apps: { l: '<rect x="4" y="4" width="6.5" height="6.5" rx="2.6"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="2.6"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="2.6"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="2.6"/>',
    f: '<rect x="4" y="4" width="6.5" height="6.5" rx="2.6"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="2.6"/>' },
  touchpad: { l:'<rect x="3" y="4" width="18" height="16" rx="3.5"/><path d="M3 15h18M12 15v5"/><circle cx="12" cy="9.5" r="1.2"/>',
    f: '<rect x="3" y="4" width="18" height="16" rx="3.5"/>' }
};
// ---- Rucne urcene vyplne tam, kde automatika nevybere dobrou plochu.
const FILL = {
  settings: '<circle cx="12" cy="12" r="7"/>', tasks: '<rect x="3" y="4" width="5" height="16" rx="2"/>', search: '<circle cx="11" cy="11" r="6.5"/>',
  trash: '<path d="M6.5 7l1 13h9l1-13z"/>', wifi: '<path d="M5.5 12.5a9.5 9.5 0 0 1 13 0L12 19z"/>', volume: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/>',
  user: '<circle cx="12" cy="8.5" r="4"/><path d="M4.5 20a7.5 7.5 0 0 1 15 0z"/>', lock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/>',
  disk: '<path d="M3 13h18v3a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z"/>', history: '<circle cx="12" cy="12" r="7.5"/>', clock: '<circle cx="12" cy="12" r="8"/>',
  branch: '<circle cx="6" cy="5.5" r="2"/><circle cx="6" cy="18.5" r="2"/><circle cx="18" cy="8" r="2"/>', bell: '<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/>',
  apps: '<rect x="4" y="4" width="6.5" height="6.5" rx="2"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="2"/>', globe: '<circle cx="12" cy="12" r="9"/>'
};

// Automaticka vypln: prvni rect, kruh (r >= 2.5) nebo uzavrena cesta.
function autoFill(body) {
  const els = body.match(/<(rect|circle|path)\b[^>]*\/>/g) || [];
  for (const e of els) {
    if (/fill="currentColor"/.test(e)) continue;
    if (e.startsWith('<rect')) return e;
    if (e.startsWith('<circle')) { const r = +(e.match(/\br="([\d.]+)"/) || [0, 0])[1]; if (r >= 2.5) return e; }
    if (e.startsWith('<path') && /[zZ]"/.test(e)) return e;
  }
  return '';
}

const files = readdirSync(src).filter((f) => f.endsWith('.svg')).sort();
const out = {};
for (const f of files) {
  const name = f.replace(/\.svg$/, '');
  const svg = readFileSync(join(src, f), 'utf8');
  if (/<script|on\w+=|href=/i.test(svg)) throw new Error(name + ': nepovoleny obsah v SVG');
  const body = svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>[\s\S]*$/, '').trim();
  const c = catOf[name] || 'system';
  const r = REDRAW[name];
  const l = r ? r.l : body;
  const fill = c === 'ui' ? '' : r ? r.f : (FILL[name] || autoFill(body));
  out[name] = { c, f: fill, l };
}

const counts = {};
for (const v of Object.values(out)) counts[v.c] = (counts[v.c] || 0) + 1;
const block = '  // ICONS-START (vygenerovano: node design/tools/build-icons.mjs; neupravovat rucne)\n' +
  '  var ICONS = ' + JSON.stringify(out) + ';\n  // ICONS-END';
const moPath = join(here, '..', 'assets', 'mo.js');
let mo = readFileSync(moPath, 'utf8');
const a = mo.indexOf('  // ICONS-START'), b = mo.indexOf('// ICONS-END');
if (a < 0 || b < 0) throw new Error('mo.js nema znacky ICONS-START / ICONS-END');
mo = mo.slice(0, a) + block + mo.slice(b + '// ICONS-END'.length);
writeFileSync(moPath, mo);
console.log('ikon: ' + Object.keys(out).length, JSON.stringify(counts));
