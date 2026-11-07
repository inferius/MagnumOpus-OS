// Spolecny skript prototypu: sada ikon, prototypova lista, motiv, kontrast,
// velikost pisma a omezeni pohybu. Neni to kod systemu - jen nastroj pro nahled.
(function () {
  'use strict';
  var root = document.documentElement;

  // ---- Ikony: 24x24, tah 1.75 (styl "linkove, zaoblene"). Jen cesty, barvu da CSS.
  var ICONS = {
    terminal: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 9l3 3-3 3M12.5 15H17"/>',
    folder: '<path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z"/>',
    file: '<path d="M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M14 3v5h5"/>',
    code: '<path d="M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M10 12l-2 2 2 2M14 12l2 2-2 2"/>',
    image: '<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="9.5" r="1.5"/><path d="M21 16l-5-5-8 9"/>',
    settings: '<circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7 7 0 0 0-2-1.2L14 3h-4l-.6 2.6a7 7 0 0 0-2 1.2l-2.3-.9-2 3.4 2 1.5a7 7 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-.9a7 7 0 0 0 2 1.2L10 21h4l.6-2.6a7 7 0 0 0 2-1.2l2.3.9 2-3.4-2-1.5c.1-.4.1-.8.1-1.2z"/>',
    shield: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/>',
    shieldcheck: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/><path d="M8.5 12l2.5 2.5 4.5-5"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.8 3.2 2.8 14.8 0 18M12 3c-2.8 3.2-2.8 14.8 0 18"/>',
    camera: '<path d="M4 8.5A2.5 2.5 0 0 1 6.5 6H8l1.5-2h5L16 6h1.5A2.5 2.5 0 0 1 20 8.5v8a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 16.5z"/><circle cx="12" cy="12.5" r="3.5"/>',
    mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
    lock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>',
    unlock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 7.7-1.5"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 7l2 2M14 9l1.5 1.5"/>',
    user: '<circle cx="12" cy="8.5" r="4"/><path d="M4.5 20a7.5 7.5 0 0 1 15 0"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    bell: '<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
    tasks: '<path d="M10 6.5h10M10 12h10M10 17.5h10"/><path d="M3.5 6.5l1.5 1.5 2.5-3M3.5 12l1.5 1.5 2.5-3"/><circle cx="5.5" cy="17.5" r="1.5"/>',
    agent: '<path d="M12 3l1.8 4.6L18.5 9l-4.7 1.8L12 15.5l-1.8-4.7L5.5 9l4.7-1.4z"/><path d="M18 15l.9 2.1L21 18l-2.1.9L18 21l-.9-2.1L15 18l2.1-.9z"/>',
    close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    min: '<path d="M6 12h12"/>',
    max: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
    restore: '<rect x="5" y="9" width="10" height="10" rx="2"/><path d="M9 9V7a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-2"/>',
    right: '<path d="M9.5 6l6 6-6 6"/>',
    left: '<path d="M14.5 6l-6 6 6 6"/>',
    down: '<path d="M6 9.5l6 6 6-6"/>',
    up: '<path d="M6 14.5l6-6 6 6"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    alert: '<path d="M12 4l9 16H3z"/><path d="M12 10v4.5M12 17.5v.5"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>',
    history: '<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6"/><path d="M3.5 4.5V8H7"/><path d="M12 7.5V12l3 2"/>',
    branch: '<circle cx="6" cy="5.5" r="2"/><circle cx="6" cy="18.5" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 7.5v9M18 10c0 4-6 3-11.2 7"/>',
    undo: '<path d="M9 7L4.5 11.5 9 16"/><path d="M4.5 11.5H15a5 5 0 0 1 0 10h-3"/>',
    play: '<path d="M8 5.5v13l10-6.5z"/>',
    pause: '<path d="M8.5 5.5v13M15.5 5.5v13"/>',
    stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/>',
    trash: '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 13h9l1-13"/>',
    copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M15.5 8.5V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7.5a2 2 0 0 0 2 2h2.5"/>',
    eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>',
    keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6 10h.5M9.5 10h.5M13 10h.5M16.5 10h1M7 14.5h10"/>',
    usb: '<path d="M12 21V5M12 5l-2.5 3M12 5l2.5 3"/><path d="M12 15l-4-2.5V10M12 13l4-2V8.5"/><rect x="6.5" y="8" width="3" height="2"/><circle cx="16" cy="7.5" r="1.2"/>',
    power: '<path d="M12 3.5V12"/><path d="M7 6.5a7.5 7.5 0 1 0 10 0"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
    contrast: '<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor"/>',
    textsize: '<path d="M3 18l4.5-12L12 18M4.6 14h5.8M14 18l3.3-8.5L20.5 18M15 15.5h4.6"/>',
    apps: '<rect x="4" y="4" width="6.5" height="6.5" rx="2"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="2"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="2"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="2"/>',
    monitor: '<rect x="3" y="4" width="18" height="12.5" rx="2.5"/><path d="M8.5 20.5h7M12 16.5v4"/>',
    wifi: '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19" r=".8"/>',
    volume: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
    battery: '<rect x="2.5" y="7.5" width="17" height="9" rx="2.5"/><path d="M21.5 10.5v3"/><path d="M5.5 10.5h8v3h-8z" fill="currentColor"/>',
    download: '<path d="M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14"/>',
    home: '<path d="M4 11l8-6.5 8 6.5v8.5a1.5 1.5 0 0 1-1.5 1.5H15v-6H9v6H5.5A1.5 1.5 0 0 1 4 19.5z"/>',
    more: '<circle cx="6" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18" cy="12" r="1.2"/>',
    refresh: '<path d="M20 11.5a8 8 0 0 0-14.5-4.2M4 12.5a8 8 0 0 0 14.5 4.2"/><path d="M5 3.5v4h4M19 20.5v-4h-4"/>',
    filter: '<path d="M4 5.5h16l-6 7.5v5.5l-4 2V13z"/>',
    sidebar: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>',
    window: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18"/>',
    cloud: '<path d="M7 18.5a4.5 4.5 0 0 1-.5-9 6 6 0 0 1 11.6 1.5A3.75 3.75 0 0 1 17.5 18.5z"/>',
    disk: '<rect x="3" y="5" width="18" height="14" rx="3"/><path d="M3 13h18"/><circle cx="17" cy="16" r=".8"/>',
    pin: '<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z"/><path d="M12 14v6.5"/>',
    star: '<path d="M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.9-5.2-2.8-5.2 2.8 1-5.9-4.3-4.1 5.9-.8z"/>',
    send: '<path d="M4 12l16-7.5-6 16-2.5-6.5z"/><path d="M11.5 14l8.5-9.5"/>',
    printer: '<path d="M7 9V4h10v5"/><rect x="3.5" y="9" width="17" height="8" rx="2.5"/><path d="M7 14h10v6H7z"/>',
    accessibility: '<circle cx="12" cy="4.5" r="1.8"/><path d="M5 8.5l7 1.5 7-1.5M12 10v4.5l-3.5 6M12 14.5l3.5 6"/>',
    palette: '<path d="M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.9 1.4-2-.5-1.3.3-2.5 1.7-2.5H17a4 4 0 0 0 4-4C21 6.9 17 3 12 3z"/><circle cx="7.5" cy="11.5" r="1"/><circle cx="10" cy="7.5" r="1"/><circle cx="14.5" cy="7.5" r="1"/>',
    network: '<rect x="9" y="3" width="6" height="5" rx="1.5"/><rect x="3" y="16" width="6" height="5" rx="1.5"/><rect x="15" y="16" width="6" height="5" rx="1.5"/><path d="M12 8v4M6 16v-2h12v2"/>',
    gauge: '<path d="M4.5 17a8.5 8.5 0 1 1 15 0"/><path d="M12 13l3.5-4"/><circle cx="12" cy="13" r="1.2"/>',
    diff: '<path d="M8 4v8M4 8h8M13 18h8"/>'
  };
  // Logo: kapka/oblouk ve fialove - vlastni znak, ne prevzaty.
  var LOGO = '<svg viewBox="0 0 32 32" class="logo" aria-hidden="true"><rect width="32" height="32" rx="10" fill="var(--c-accent)"/>' +
    '<path d="M8 21V12.5a3 3 0 0 1 6 0V21M14 12.5a3 3 0 0 1 6 0V21" fill="none" stroke="var(--c-on-accent)" stroke-width="2.4" stroke-linecap="round"/>' +
    '<circle cx="23.5" cy="20" r="2.4" fill="var(--c-on-accent)"/></svg>';
  window.MO_LOGO = LOGO;

  function injectSprite() {
    var s = '<svg xmlns="http://www.w3.org/2000/svg" style="position:absolute;width:0;height:0" aria-hidden="true">';
    Object.keys(ICONS).forEach(function (k) { s += '<symbol id="ic-' + k + '" viewBox="0 0 24 24">' + ICONS[k] + '</symbol>'; });
    s += '</svg>';
    document.body.insertAdjacentHTML('afterbegin', s);
  }
  window.MO_ICONS = Object.keys(ICONS);
  // Pomocnik pro sablony: ic('folder', 's')
  window.ic = function (name, cls) {
    return '<svg class="ic ' + (cls || '') + '" aria-hidden="true"><use href="#ic-' + name + '"/></svg>';
  };

  // ---- Nastaveni nahledu (uklada se, kdyz uloziste jde).
  function load(k, d) { try { return localStorage.getItem('mo.' + k) || d; } catch (e) { return d; } }
  function save(k, v) { try { localStorage.setItem('mo.' + k, v); } catch (e) {} }
  var state = {
    theme: load('theme', 'auto'),       // auto | light | dark
    contrast: load('contrast', 'normal'),
    scale: load('scale', '1'),
    motion: load('motion', 'system')    // system | reduce | full
  };
  // "Podle denni doby": svetly 7:00-19:59, jinak tmavy. System to bude delat
  // podle vychodu a zapadu slunce v nastavenem pasmu; tady jen priblizne.
  function daytime() { var h = new Date().getHours(); return h >= 7 && h < 20 ? 'light' : 'dark'; }
  function apply() {
    root.setAttribute('data-theme', state.theme === 'auto' ? daytime() : state.theme);
    if (state.contrast === 'more') root.setAttribute('data-contrast', 'more'); else root.removeAttribute('data-contrast');
    root.style.setProperty('--text-scale', state.scale);
    if (state.motion === 'system') root.removeAttribute('data-motion'); else root.setAttribute('data-motion', state.motion);
    document.dispatchEvent(new CustomEvent('mo-theme'));
  }
  window.MO_SET = function (k, v) { state[k] = v; save(k, v); apply(); syncControls(); };
  window.MO_STATE = state;
  apply();

  var PAGES = [
    ['index.html', 'Přehled'], ['design-system.html', 'Design systém'], ['okna.html', 'Okna'],
    ['plocha.html', 'Plocha'], ['terminal.html', 'Terminál'], ['soubory.html', 'Soubory'],
    ['nastaveni.html', 'Nastavení'], ['prihlaseni.html', 'Přihlášení']
  ];
  function syncControls() {
    var m = { theme: 'mo-theme-sel', contrast: 'mo-contrast-sel', scale: 'mo-scale-sel', motion: 'mo-motion-sel' };
    Object.keys(m).forEach(function (k) { var el = document.getElementById(m[k]); if (el) el.value = state[k]; });
  }
  function protoBar() {
    var here = location.pathname.split('/').pop() || 'index.html';
    var nav = PAGES.map(function (p) {
      return '<a href="' + p[0] + '"' + (p[0] === here ? ' aria-current="page"' : '') + '>' + p[1] + '</a>';
    }).join('');
    var html = '<div class="proto" role="region" aria-label="Ovládání náhledu (není součást systému)">' +
      '<span class="brand">' + LOGO + 'MagnumOpus-OS <span class="muted" style="font-weight:400">prototyp</span></span>' +
      '<nav aria-label="Prototypy">' + nav + '</nav>' +
      '<div class="ctl">' +
      '<label>' + window.ic('palette', 's') + '<select id="mo-theme-sel" aria-label="Motiv"><option value="auto">Podle denní doby</option><option value="light">Světlý</option><option value="dark">Tmavý</option></select></label>' +
      '<label>' + window.ic('contrast', 's') + '<select id="mo-contrast-sel" aria-label="Kontrast"><option value="normal">Běžný kontrast</option><option value="more">Vysoký kontrast</option></select></label>' +
      '<label>' + window.ic('textsize', 's') + '<select id="mo-scale-sel" aria-label="Velikost písma"><option value="1">100 %</option><option value="1.25">125 %</option><option value="1.5">150 %</option><option value="2">200 %</option></select></label>' +
      '<label>' + window.ic('gauge', 's') + '<select id="mo-motion-sel" aria-label="Pohyb"><option value="system">Pohyb podle systému</option><option value="reduce">Omezit pohyb</option><option value="full">Plný pohyb</option></select></label>' +
      '</div></div>';
    var host = document.getElementById('proto');
    if (host) host.outerHTML = html; else document.body.insertAdjacentHTML('afterbegin', html);
    [['mo-theme-sel', 'theme'], ['mo-contrast-sel', 'contrast'], ['mo-scale-sel', 'scale'], ['mo-motion-sel', 'motion']].forEach(function (p) {
      document.getElementById(p[0]).addEventListener('change', function (e) { window.MO_SET(p[1], e.target.value); });
    });
    syncControls();
  }

  // ---- Pomocnik: radiogroup .seg ovladatelny sipkami (role="radio").
  window.MO_SEG = function (el, onChange) {
    var btns = Array.prototype.slice.call(el.querySelectorAll('[role="radio"]'));
    function pick(b, focus) {
      btns.forEach(function (x) { var on = x === b; x.setAttribute('aria-checked', on); x.tabIndex = on ? 0 : -1; });
      if (focus) b.focus();
      if (onChange) onChange(b.dataset.value, b);
    }
    btns.forEach(function (b, i) {
      b.tabIndex = b.getAttribute('aria-checked') === 'true' ? 0 : -1;
      b.addEventListener('click', function () { pick(b); });
      b.addEventListener('keydown', function (e) {
        var d = (e.key === 'ArrowRight' || e.key === 'ArrowDown') ? 1 : (e.key === 'ArrowLeft' || e.key === 'ArrowUp') ? -1 : 0;
        if (d) { e.preventDefault(); pick(btns[(i + d + btns.length) % btns.length], true); }
      });
    });
  };
  // ---- Pomocnik: prepinac role="switch".
  window.MO_SWITCH = function (el, onChange) {
    el.addEventListener('click', function () {
      var v = el.getAttribute('aria-checked') !== 'true';
      el.setAttribute('aria-checked', v);
      if (onChange) onChange(v, el);
    });
  };
  // ---- Pomocnik: drzeni fokusu v dialogu (Tab cykli uvnitr, Esc = posledni volba).
  window.MO_TRAP = function (dlg, onEsc) {
    function items() {
      return Array.prototype.slice.call(dlg.querySelectorAll('button:not([disabled]), input, select, [tabindex="0"], a[href]'));
    }
    dlg.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && onEsc) { e.preventDefault(); onEsc(); return; }
      if (e.key !== 'Tab') return;
      var it = items(); if (!it.length) return;
      var first = it[0], last = it[it.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    });
  };

  function boot() { injectSprite(); protoBar(); document.dispatchEvent(new CustomEvent('mo-ready')); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot); else boot();
})();
