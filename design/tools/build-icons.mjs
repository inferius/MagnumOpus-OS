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

// ---- Doplnene ikony (nejsou ve zdrojove sade): jmeno -> [kategorie, kresba, vypln].
const D = '<rect x="3" y="5" width="18" height="14" rx="4"/>';
const DOC = '<path d="M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/>';
const WIN = '<rect x="3" y="4" width="18" height="16" rx="3.5"/>';
const SHIELD = '<path d="M12 3.2l6.2 2.4a2 2 0 0 1 1.3 1.9v4c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5v-4a2 2 0 0 1 1.3-1.9z"/>';
const SPARK = (x, y, s) => '<path d="M' + x + ' ' + (y - 3 * s) + 'l' + (.9 * s) + ' ' + (2.1 * s) + ' ' + (2.1 * s) + ' ' + (.9 * s) + '-' + (2.1 * s) + ' ' + (.9 * s) + '-' + (.9 * s) + ' ' + (2.1 * s) + '-' + (.9 * s) + '-' + (2.1 * s) + '-' + (2.1 * s) + '-' + (.9 * s) + ' ' + (2.1 * s) + '-' + (.9 * s) + 'z"/>';
const EXTRA = {
  // FS
  dataset: ['files', '<path d="M12 4l8 4-8 4-8-4z"/><path d="M4 12l8 4 8-4M4 16l8 4 8-4"/>', '<path d="M12 4l8 4-8 4-8-4z"/>'],
  vault: ['files', '<rect x="3.5" y="4" width="17" height="15" rx="3"/><circle cx="12" cy="11.5" r="3.5"/><path d="M12 8v1M12 14v1M8.5 11.5h1M14.5 11.5h1M7 19v2M17 19v2"/>', '<rect x="3.5" y="4" width="17" height="15" rx="3"/>'],
  commit: ['time', '<circle cx="12" cy="12" r="3.5"/><path d="M3 12h5.5M15.5 12H21"/>', '<circle cx="12" cy="12" r="3.5"/>'],
  tag: ['files', '<path d="M3.5 12.6V5.5a2 2 0 0 1 2-2h7.1a2 2 0 0 1 1.4.6l6.4 6.4a2 2 0 0 1 0 2.8l-7.1 7.1a2 2 0 0 1-2.8 0L4.1 14a2 2 0 0 1-.6-1.4z"/><circle cx="8.5" cy="8.5" r="1.5"/>', '<path d="M3.5 12.6V5.5a2 2 0 0 1 2-2h7.1a2 2 0 0 1 1.4.6l6.4 6.4a2 2 0 0 1 0 2.8l-7.1 7.1a2 2 0 0 1-2.8 0L4.1 14a2 2 0 0 1-.6-1.4z"/>'],
  journal: ['time', '<rect x="5" y="3" width="14" height="18" rx="2.5"/><path d="M9 3v18M12 8h4M12 12h4"/>', '<rect x="5" y="3" width="14" height="18" rx="2.5"/>'],
  'snapshot-locked': ['time', '<rect x="3" y="6" width="13" height="12" rx="3"/><circle cx="9.5" cy="12" r="2.5"/><rect x="15.5" y="13.5" width="6" height="6" rx="1.5"/><path d="M16.8 13.5v-1.3a1.7 1.7 0 0 1 3.4 0v1.3"/>', '<rect x="3" y="6" width="13" height="12" rx="3"/>'],
  quota: ['files', '<path d="M4 17a8 8 0 1 1 16 0z"/><path d="M12 17l4-5"/>', '<path d="M4 17a8 8 0 1 1 16 0z"/>'],
  dedup: ['files', '<rect x="3.5" y="3.5" width="11" height="13" rx="2.5"/><rect x="9.5" y="7.5" width="11" height="13" rx="2.5"/><path d="M13 12.5h4M13 15.5h4"/>', '<rect x="9.5" y="7.5" width="11" height="13" rx="2.5"/>'],
  compress: ['files', '<rect x="5" y="3" width="14" height="18" rx="3"/><path d="M12 3v2.5M12 7.5v2M12 11.5v2"/><rect x="10.5" y="15" width="3" height="3" rx="1"/>', '<rect x="5" y="3" width="14" height="18" rx="3"/>'],
  scrub: ['files', D + '<path d="M7 12l2.5 2.5L14 10"/><path d="M17 9v6"/>', D],
  // Disky
  ssd: ['files', '<rect x="3" y="6" width="18" height="12" rx="3"/><path d="M7 10h6M7 14h3M17 10v4"/>', '<rect x="3" y="6" width="18" height="12" rx="3"/>'],
  hdd: ['files', '<rect x="4" y="3" width="16" height="18" rx="3"/><circle cx="12" cy="10.5" r="4"/><path d="M12 10.5l3 5"/><circle cx="16.5" cy="17.5" r=".8"/>', '<rect x="4" y="3" width="16" height="18" rx="3"/>'],
  partition: ['files', '<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5V12l6 6"/>', '<path d="M12 3.5A8.5 8.5 0 0 1 18 18l-6-6z"/>'],
  'disk-foreign': ['files', D + '<path d="M10 10a2 2 0 1 1 2.7 1.9c-.4.2-.7.5-.7 1v.3M12 15.5v.01"/>', D],
  format: ['files', D + '<path d="M8 15l6-6M11 15h5"/>', D],
  smart: ['files', D + '<path d="M6 12h3l1.5-3 3 6 1.5-3h3"/>', D],
  'flash-drive': ['files', '<rect x="2.5" y="8" width="13" height="8" rx="2.5"/><path d="M15.5 9.5h4a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1h-4M17.5 11.3v.01M17.5 12.7v.01"/>', '<rect x="2.5" y="8" width="13" height="8" rx="2.5"/>'],
  'sd-card': ['files', '<path d="M8 3h9a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6z"/><path d="M9 3.5v3.5M12 3.5v3.5M15 3.5v3.5"/>', '<path d="M8 3h9a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6z"/>'],
  // System
  driver: ['system', '<rect x="6" y="6" width="12" height="12" rx="2.5"/><circle cx="12" cy="12" r="2.5"/><path d="M9 2.5V6M15 2.5V6M9 18v3.5M15 18v3.5M2.5 9H6M2.5 15H6M18 9h3.5M18 15h3.5"/>', '<rect x="6" y="6" width="12" height="12" rx="2.5"/>'],
  service: ['system', D + '<circle cx="12" cy="12" r="3"/><path d="M12 7.5V9M12 15v1.5M7.5 12H9M15 12h1.5"/>', D],
  process: ['system', '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18M7 13h4M7 16h7"/>', '<path d="M6 4h12a3 3 0 0 1 3 3v2H3V7a3 3 0 0 1 3-3z"/>'],
  'crash-recovery': ['system', '<path d="M4 12a8 8 0 1 0 2.3-5.7"/><path d="M4 4.5V8h3.5"/><path d="M12.5 8l-2 4h3l-2 4"/>', '<circle cx="12" cy="12" r="7"/>'],
  'crash-log': ['system', DOC + '<path d="M12.5 10l-2 3.5h3l-2 3.5"/>', DOC],
  report: ['system', DOC + '<path d="M9 17v-3M12 17v-5M15 17v-2"/>', DOC],
  firewall: ['network', '<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M3 9.7h18M3 14.3h18M9 5v4.7M15 9.7v4.6M9 14.3V19M15 5v4.7"/>', '<rect x="3" y="5" width="18" height="14" rx="2.5"/>'],
  certificate: ['system', '<rect x="3" y="4" width="18" height="13" rx="2.5"/><path d="M7 8.5h10M7 12h5"/><circle cx="16" cy="15" r="2.5"/><path d="M14.8 17.3L14 21l2-1 2 1-.8-3.7"/>', '<rect x="3" y="4" width="18" height="13" rx="2.5"/>'],
  dns: ['network', '<circle cx="9" cy="9" r="6"/><path d="M3 9h12M9 3c1.8 2 1.8 10 0 12M14 17h7M14 20.5h5M17 13.5h4"/>', '<circle cx="9" cy="9" r="6"/>'],
  'time-sync': ['time', '<circle cx="12" cy="12" r="7"/><path d="M12 8.5V12l2.5 1.5M19.5 5v3h-3M4.5 19v-3h3"/>', '<circle cx="12" cy="12" r="7"/>'],
  sandbox: ['system', '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M4 7.5l8 4.5 8-4.5M12 12v9"/>', '<path d="M12 3l8 4.5-8 4.5-8-4.5z"/>'],
  jit: ['system', '<path d="M7 5L3 12l4 7M17 5l4 7-4 7"/><path d="M13 5l-3 7h4l-3 7"/>', ''],
  // HW
  gpu: ['devices', '<rect x="2.5" y="6" width="19" height="11" rx="2.5"/><circle cx="9" cy="11.5" r="3"/><path d="M15 9.5h3M15 13.5h3M5 17v2.5M9 17v2.5"/>', '<rect x="2.5" y="6" width="19" height="11" rx="2.5"/>'],
  'multi-monitor': ['devices', '<rect x="2.5" y="4" width="12" height="9" rx="2"/><rect x="9.5" y="9" width="12" height="9" rx="2"/><path d="M13.5 21h4M15.5 18v3"/>', '<rect x="9.5" y="9" width="12" height="9" rx="2"/>'],
  'audio-output': ['devices', '<rect x="5" y="3" width="14" height="18" rx="3"/><circle cx="12" cy="14" r="3.5"/><circle cx="12" cy="7.5" r="1"/>', '<rect x="5" y="3" width="14" height="18" rx="3"/>'],
  temperature: ['devices', '<path d="M10 14.5V5a2 2 0 0 1 4 0v9.5a4 4 0 1 1-4 0z"/><path d="M12 9v7"/>', '<path d="M10 14.5V5a2 2 0 0 1 4 0v9.5a4 4 0 1 1-4 0z"/>'],
  nfc: ['network', '<path d="M6 7.5a8 8 0 0 1 0 9M9.5 9.5a4.5 4.5 0 0 1 0 5"/><rect x="13" y="4" width="8" height="16" rx="2.5"/>', '<rect x="13" y="4" width="8" height="16" rx="2.5"/>'],
  pen: ['devices', '<path d="M15.5 4.5l4 4L9 19l-5 1 1-5z"/><path d="M13.5 6.5l4 4"/>', '<path d="M15.5 4.5l4 4L9 19l-5 1 1-5z"/>'],
  gamepad: ['devices', '<path d="M7 7h10a4.5 4.5 0 0 1 4.4 5.5l-1 4.2a2.3 2.3 0 0 1-3.9 1L14.5 16h-5l-2 1.7a2.3 2.3 0 0 1-3.9-1l-1-4.2A4.5 4.5 0 0 1 7 7z"/><path d="M8 10.5v3M6.5 12h3M15.5 11h.01M17.5 13h.01"/>', '<path d="M7 7h10a4.5 4.5 0 0 1 4.4 5.5l-1 4.2a2.3 2.3 0 0 1-3.9 1L14.5 16h-5l-2 1.7a2.3 2.3 0 0 1-3.9-1l-1-4.2A4.5 4.5 0 0 1 7 7z"/>'],
  // Ucty
  'pin-code': ['system', '<rect x="3" y="6" width="18" height="12" rx="3"/><path d="M7.5 12h.01M10.5 12h.01M13.5 12h.01M16.5 12h.01"/>', '<rect x="3" y="6" width="18" height="12" rx="3"/>'],
  fido2: ['system', '<rect x="8" y="2.5" width="8" height="9" rx="2"/><path d="M6.5 11.5h11v6a4 4 0 0 1-4 4h-3a4 4 0 0 1-4-4z"/><circle cx="12" cy="16.5" r="1.5"/>', '<path d="M6.5 11.5h11v6a4 4 0 0 1-4 4h-3a4 4 0 0 1-4-4z"/>'],
  'recovery-key': ['system', '<circle cx="8" cy="15" r="4"/><path d="M11 12l6-6M15 8l2 2M14 3.5a6 6 0 0 1 6.5 6.5"/>', '<circle cx="8" cy="15" r="4"/>'],
  elevate: ['system', SHIELD + '<path d="M12 16V9M9 11.5l3-3 3 3"/>', SHIELD],
  'child-account': ['system', '<circle cx="9" cy="8" r="3.5"/><path d="M3 19.5a6 6 0 0 1 12 0"/><circle cx="17" cy="11" r="2.5"/><path d="M14.5 19.5a4 4 0 0 1 7 0"/>', '<circle cx="17" cy="11" r="2.5"/>'],
  // Agent
  'agent-workspace': ['agent', '<path d="M3 8a3 3 0 0 1 3-3h3.6a2 2 0 0 1 1.4.6L12.5 7H18a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z"/>' + SPARK(12, 13, 1), '<path d="M3 8a3 3 0 0 1 3-3h3.6a2 2 0 0 1 1.4.6L12.5 7H18a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z"/>'],
  'agent-request': ['agent', '<path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v8a2.5 2.5 0 0 1-2.5 2.5H11l-4 3.5V17h-.5A2.5 2.5 0 0 1 4 14.5z"/>' + SPARK(12, 10.5, 1), '<path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v8a2.5 2.5 0 0 1-2.5 2.5H11l-4 3.5V17h-.5A2.5 2.5 0 0 1 4 14.5z"/>'],
  'agent-undo': ['agent', '<path d="M9 7L4.5 11.5 9 16"/><path d="M4.5 11.5H14a5 5 0 0 1 0 10h-2"/>' + SPARK(17.5, 5.5, .8), SPARK(17.5, 5.5, .8)],
  'agent-paused': ['agent', SPARK(9.5, 9, 1.6) + '<rect x="14.5" y="14" width="2.5" height="7" rx="1"/><rect x="19" y="14" width="2.5" height="7" rx="1"/>', SPARK(9.5, 9, 1.6)],
  // Aplikace
  'app-browser': ['network', WIN + '<path d="M3 8.5h18"/><circle cx="12" cy="14" r="3.5"/><path d="M8.5 14h7"/>', WIN],
  'app-editor': ['system', WIN + '<path d="M7 9h6M7 12.5h4M14 17.5l4.5-4.5 1.5 1.5-4.5 4.5H14z"/>', WIN],
  'app-player': ['devices', WIN + '<path d="M10 9v6l5-3z"/>', WIN],
  'app-pdf': ['files', DOC + '<path d="M8 17v-4h1.5a1.2 1.2 0 0 1 0 2.4H8M12 13v4h1a2 2 0 0 0 0-4zM16 17v-4h2M16 15h1.5"/>', DOC],
  'app-photos': ['devices', WIN + '<circle cx="9" cy="9.5" r="1.8"/><path d="M21 15.5l-5-5-7.5 9.5"/>', WIN],
  calculator: ['system', '<rect x="5" y="3" width="14" height="18" rx="3"/><rect x="8" y="6" width="8" height="3.5" rx="1"/><path d="M8.5 13h.01M12 13h.01M15.5 13h.01M8.5 16.5h.01M12 16.5h.01M15.5 16.5h.01"/>', '<rect x="5" y="3" width="14" height="18" rx="3"/>'],
  'disk-manager': ['files', D + '<path d="M3 13h18"/><circle cx="17" cy="16" r=".9"/><path d="M7 9h5"/>', '<path d="M3 13h18v2a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4z"/>'],
  'app-installer': ['files', '<path d="M4 9l8-4.5L20 9v8.5L12 22l-8-4.5z"/><path d="M12 9v6M9.5 12.5L12 15l2.5-2.5"/>', '<path d="M4 9l8-4.5L20 9v8.5L12 22l-8-4.5z"/>'],
  git: ['time', '<circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 8v8M8 6h4a4 4 0 0 1 4 4"/>', '<circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="12" r="2"/>'],
  'windows-exe': ['files', WIN + '<path d="M3 8.5h18M7 12.5h3M7 14.5h2.5M7 16.5h3M7 12.5v4M13 12.5l3 4M16 12.5l-3 4"/>', WIN],
  // Sit
  metered: ['network', '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0"/><circle cx="12" cy="17.5" r="3"/><path d="M12 16.3v2.4"/>', '<circle cx="12" cy="17.5" r="3"/>'],
  proxy: ['network', '<circle cx="5" cy="12" r="2"/><circle cx="19" cy="12" r="2"/><rect x="9" y="8.5" width="6" height="7" rx="2"/><path d="M7 12h2M15 12h2"/>', '<rect x="9" y="8.5" width="6" height="7" rx="2"/>'],
  'net-sharing': ['network', '<circle cx="12" cy="12" r="2.5"/><path d="M7.5 7.5a6.5 6.5 0 0 0 0 9M16.5 7.5a6.5 6.5 0 0 1 0 9M4.5 4.5a10.5 10.5 0 0 0 0 15M19.5 4.5a10.5 10.5 0 0 1 0 15"/>', '<circle cx="12" cy="12" r="2.5"/>'],
  'mobile-data': ['network', '<path d="M5 20v-3M9.5 20v-6M14 20v-9M18 14V3.5M15.5 6L18 3.5 20.5 6"/>', ''],
  // Ostatni
  'keyboard-layout': ['devices', '<rect x="2.5" y="10" width="19" height="10" rx="2.5"/><path d="M6 13.5h.5M9.5 13.5h.5M13 13.5h.5M16.5 13.5h1M7 17h10M9 3.5h6M12 3.5v4"/>', '<rect x="2.5" y="10" width="19" height="10" rx="2.5"/>'],
  snooze: ['network', '<path d="M5 15.5V11a6 6 0 0 1 9-5.2M5 15.5L3.5 17.5h13M8.5 20.5a2 2 0 0 0 4 0M15 3.5h5l-5 5h5"/>', ''],
  'power-profile': ['devices', '<path d="M4.5 17a8.5 8.5 0 1 1 15 0z"/><path d="M12 13l3.5-4"/><circle cx="12" cy="13" r="1.2"/>', '<path d="M4.5 17a8.5 8.5 0 1 1 15 0z"/>'],
  'notation-bc': ['system', '<path d="M3.5 3.5h3a2 2 0 0 1 0 4h-3zM3.5 7.5h3.5a2 2 0 0 1 0 4H3.5z"/><path d="M20.5 14.5a3.5 3.5 0 1 0 0 4.5M10 16h4M12.5 14.5L14 16l-1.5 1.5"/>', '']
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

for (const [name, [c, l, f]] of Object.entries(EXTRA)) {
  if (out[name]) throw new Error(name + ': uz je ve zdrojove sade');
  out[name] = { c, f, l };
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
