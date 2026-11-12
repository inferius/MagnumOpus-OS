/* Simulator mobilniho prostredi pro design/mobil.html.
 * Stav S popisuje zarizeni, obrazovku a prekryti; render() z nej vzdy sestavi cele HTML.
 * Neni to runtime GUI, jen prototyp pro posouzeni vzhledu a pruchodu. */
(function () {
  'use strict';

  var DEV = {
    narrow: { n: 'Úzký telefon 360 &times; 760', w: 360, h: 760, z: 0.82, scale: 3 },
    wide: { n: 'Široký telefon 412 &times; 915', w: 412, h: 915, z: 0.74, scale: 2.6 },
    tablet: { n: 'Tablet 1180 &times; 820', w: 1180, h: 820, z: 0.56, scale: 2, tab: true }
  };
  var S = {
    dev: 'narrow', land: false, big: false, areas: false, nav: 'gest', hand: 'none',
    scr: 'lock', from: 'home', kbd: false, shift: false, sel: false, sheet: null, shade: false,
    pin: -1, q: 'zpr', typed: '', cam: false, agent: true, gen: 1, step: 0, fileSel: 'zprava.md'
  };

  function I(n, c) { return '<svg class="ic' + (c ? ' ' + c : '') + '" aria-hidden="true"><use href="#ic-' + n + '"/></svg>'; }
  function A(a, v) { return ' data-a="' + a + '"' + (v !== undefined ? ' data-v="' + v + '"' : ''); }
  function esc(s) { return s.replace(/&/g, '&amp;').replace(/</g, '&lt;'); }
  function norm(s) { return s.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, ''); }

  var APPS = { files: ['folder', 'Soubory'], editor: ['app-editor', 'Editor'], terminal: ['terminal', 'Terminál'], photos: ['app-photos', 'Obrázky'],
    browser: ['app-browser', 'Prohlížeč'], settings: ['settings', 'Nastavení'], calc: ['calculator', 'Kalkulačka'], agent: ['agent', 'Agent'],
    phone: ['mobile-data', 'Telefon', 1], msg: ['send', 'Zprávy', 1] };
  var FILES = [['folder', 'Projekty', 'složka &middot; 12 položek'], ['file-text', 'zprava.md', 'před 2 min &middot; 4 kB'], ['file-text', 'zapis-z-porady.md', 'včera &middot; 2 kB'],
    ['app-pdf', 'smlouva.pdf', '3. 11. &middot; 240 kB'], ['image', 'foto.jpg', '28. 10. &middot; 3,1 MB'], ['file', 'rozpocet.ods', '2. 11. &middot; 48 kB']];
  var CATALOG = [['app', 'files'], ['app', 'editor'], ['app', 'terminal'], ['app', 'photos'], ['app', 'browser'], ['app', 'settings'], ['app', 'agent'],
    ['file', 'zprava.md', 'Dokumenty'], ['file', 'zapis-z-porady.md', 'Dokumenty'], ['file', 'smlouva.pdf', 'Dokumenty'],
    ['act', 'Zamknout', 'lock'], ['act', 'Tmavý motiv', 'moon'], ['act', 'Nový dokument', 'file-plus'], ['act', 'Připojit monitor', 'multi-monitor'], ['act', 'Zprávy: nová zpráva', 'send', 1]];
  var WORDS = ['návrh', 'návrhu', 'telefon', 'telefonu', 'rozhraní', 'průchod', 'příští', 'týden', 'dokončíme', 'ověříme', 'schůzka', 'klávesnice', 'který', 'které', 'protože', 'projekt', 'projektu', 'zítra', 'dnes', 'zpráva', 'zprávu'];

  // ---- Geometrie a kontext okna ----
  function geo() {
    var d = DEV[S.dev], W = S.land ? d.h : d.w, H = S.land ? d.w : d.h;
    var ins = d.tab ? { t: 26, r: 0, b: 20, l: 0 } : S.land ? { t: 24, r: 0, b: 16, l: 36 } : { t: 34, r: 0, b: 22, l: 0 };
    if (S.nav === 'btn') ins.b = d.tab ? 44 : 48;
    var kh = d.tab ? Math.round(H * (S.land ? 0.36 : 0.28)) : S.land ? 150 : Math.round(H * 0.36);
    var kbd = S.kbd || S.scr === 'search' || (S.scr === 'lock' && S.pin >= 0);
    var aw = W - ins.l - ins.r, ah = H - ins.t - (kbd ? kh + ins.b : ins.b);
    var cls = aw < 600 ? 'compact' : aw < 840 ? 'medium' : 'expanded';
    return { d: d, W: W, H: H, ins: ins, kh: kh, kbd: kbd, aw: aw, ah: ah, cls: cls };
  }
  function ctxJson(g) {
    var o = { gen: S.gen, size: { w: g.aw, h: g.ah }, scale: g.d.scale, 'class': g.cls,
      insets: { top: g.ins.t, right: g.ins.r, bottom: g.ins.b, left: g.ins.l },
      obscured: g.kbd ? [{ kind: 'keyboard', h: g.kh }] : [], input: ['touch'], text_scale: S.big ? 1.3 : 1, reduce_motion: false };
    // Kazdy klic na jednom radku, vnorene objekty zhustene.
    return '{\n' + Object.keys(o).map(function (k) { return '  "' + k + '": ' + JSON.stringify(o[k]).replace(/,"/g, ', "').replace(/":/g, '": '); }).join(',\n') + '\n}';
  }

  // ---- Obrazovky ----
  function lock() {
    var pin = S.pin >= 0;
    return '<div class="lock"><div class="clk">14:05</div><div class="dt">čtvrtek 12. listopadu</div>' +
      (pin ? '<div style="margin-top:20px">Zadejte PIN</div><div class="dots" style="display:flex;gap:12px;margin:10px">' + [0, 1, 2, 3].map(function (i) { return '<i style="width:12px;height:12px;border-radius:50%;border:2px solid var(--c-ink);' + (i < S.pin ? 'background:var(--c-ink)' : '') + '"></i>'; }).join('') + '</div>' :
        '<div class="ncard">' + I('send') + '<div><b>Pošta</b><small>2 nové zprávy &middot; obsah se ukáže po odemknutí</small></div></div>' +
        '<div class="ncard ai">' + I('agent') + '<div><b>Agent</b><small>Úkol dokončen &middot; podrobnosti po odemknutí</small></div></div>' +
        '<div class="unlock"' + A('unlock') + '><span class="fp">' + I('fingerprint') + '</span>Odemknout otiskem</div><button class="btn small ghost"' + A('pin') + '>Použít PIN</button>') + '</div>';
  }
  function appBtn(k) { var a = APPS[k]; return '<button class="app' + (a[2] ? ' sim' : '') + '"' + (a[2] ? '' : A('open', k)) + '><span class="t">' + I(a[0]) + '</span>' + a[1] + '</button>'; }
  function home(g) {
    var cols = g.cls === 'compact' ? 4 : g.cls === 'medium' ? 6 : 8;
    return '<div class="home"><div class="widget"><small>Dnes</small><b>15:00 Porada týmu</b><small>Zasedačka 2 &middot; za 55 min</small></div>' +
      '<div class="apps" style="grid-template-columns:repeat(' + cols + ',1fr)">' + ['terminal', 'photos', 'calc', 'agent', 'phone', 'msg', 'browser', 'settings'].map(appBtn).join('') + '</div>' +
      '<button class="pill"' + A('search') + '>' + I('search') + 'Hledat aplikace, soubory a akce</button>' +
      '<div class="dock">' + ['files', 'editor', 'browser', 'settings'].map(appBtn).join('') + '</div></div>';
  }
  function search() {
    var q = norm(S.q), out = { app: [], file: [], act: [] };
    CATALOG.forEach(function (c) {
      var name = c[0] === 'app' ? APPS[c[1]][1] : c[1];
      if (!q || norm(name).indexOf(q) >= 0) out[c[0]].push(c);
    });
    function grp(t, list, f) { return list.length ? '<div class="ph-h">' + t + '</div>' + list.map(f).join('') : ''; }
    var html = grp('Aplikace', out.app, function (c) { return '<div class="row"' + A('open', c[1]) + '>' + I(APPS[c[1]][0]) + '<div>' + APPS[c[1]][1] + '</div></div>'; }) +
      grp('Soubory', out.file, function (c) { return '<div class="row"' + A('res', c[1]) + '>' + I('file-text') + '<div>' + c[1] + '<small>' + c[2] + '</small></div></div>'; }) +
      grp('Akce', out.act, function (c) { return '<div class="row"' + (c[3] ? ' style="opacity:.5"' : '') + '>' + I(c[2]) + '<div>' + c[1] + (c[3] ? '<small>simulováno, bez modemu</small>' : '') + '</div></div>'; });
    return '<div class="res" style="background:var(--c-surface)">' + (html || '<div class="ph-h">Nic nenalezeno pro „' + esc(S.q) + '“</div>') + '</div>' +
      '<div style="padding:8px 10px;background:var(--c-surface)"><div class="pill" style="color:var(--c-ink)">' + I('search') + esc(S.q) + '<span class="caret"></span></div></div>';
  }
  function fileRow(f) { return '<div class="row' + (S.fileSel === f[1] ? ' on' : '') + '"' + A('file', f[1]) + '>' + I(f[0]) + '<div>' + f[1] + '<small>' + f[2] + '</small></div></div>'; }
  function preview() {
    var f = FILES.filter(function (x) { return x[1] === S.fileSel; })[0] || FILES[1];
    var md = /\.md$/.test(f[1]);
    return '<div class="prev">' + I(f[0], 'big') + '<h3 style="margin:8px 0 2px">' + f[1] + '</h3><div style="color:var(--c-ink-muted)">' + f[2] + '</div>' +
      (md ? '<p style="margin-top:12px">Dnešní schůzka proběhla v klidu. Příští týden dokončíme návrh&hellip;</p><div class="ph-h" style="padding-left:0">Historie verzí</div>' +
        '<div class="kv"><span>Teď (Editor)</span><span>aktuální</span></div><div class="kv"><span>14:02 (Terminál)</span><span><a href="#"' + A('sheet', 'hist') + '>Obnovit</a></span></div>' : '') +
      '<div style="display:flex;gap:8px;margin-top:14px"><button class="btn primary"' + (md ? A('openfile') : '') + '>Otevřít</button><button class="btn">' + I('share') + 'Sdílet</button></div></div>';
  }
  function files(g) {
    var list = '<div class="ph-h">Dokumenty</div>' + FILES.map(fileRow).join('');
    var bar = '<div class="abar"><button class="ib" aria-label="Zpět"' + A('back') + '>' + I('left') + '</button><h3>Dokumenty</h3><button class="ib">' + I('search') + '</button><button class="ib">' + I('more') + '</button></div>';
    if (g.cls === 'compact') return bar + '<div class="panes"><div class="pane">' + list + '</div></div>';
    var places = '<div class="pane side"><div class="ph-h">Místa</div>' + [['home', 'Domů'], ['folder', 'Dokumenty', 1], ['image', 'Obrázky'], ['download', 'Stažené'], ['history', 'Nedávné']].map(function (p) { return '<div class="row' + (p[2] ? ' on' : '') + '">' + I(p[0]) + p[1] + '</div>'; }).join('') + '</div>';
    return bar + '<div class="panes">' + (g.cls === 'expanded' ? places : '') + '<div class="pane">' + list + '</div><div class="pane">' + preview() + '</div></div>';
  }
  function docBody() {
    var t2 = 'Příští týden dokončíme ', w = 'návrh', t3 = ' mobilního rozhraní a ověříme průchod na telefonu.';
    var sel = S.sel ? '<span class="selw">' + w + '<span class="tmenu" style="bottom:calc(100% + 14px);left:-60px"><button>Vyjmout</button><button>Kopírovat</button><button>Vložit</button><button class="aib"' + A('sheet', 'agent') + '>Agent</button></span></span>' : w;
    return '<div class="doc"' + A('kbd') + '><h2>Zpráva o projektu</h2><p>Dnešní schůzka proběhla v klidu. Všichni souhlasí s plánem.</p><p>' + t2 + sel + t3 + '</p>' +
      '<p class="ai-ed">Shrnutí: termín drží, rizikem je klávesnice pro další jazyky.<span class="ai-tag">' + 'Agent</span></p>' +
      '<p>' + esc(S.typed) + (S.kbd && !S.sel ? '<span class="caret"></span>' : '') + '</p></div>';
  }
  function editor(g) {
    var bar = '<div class="abar"><button class="ib" aria-label="Zpět"' + A('back') + '>' + I('left') + '</button><h3>zprava.md</h3><span class="save">' + I('check') + 'Uloženo</span>' +
      '<button class="ib" aria-label="Zpět o krok"' + (S.typed ? '' : ' disabled') + A('undo') + '>' + I('undo') + '</button><button class="ib" aria-label="Další volby"' + A('sheet', 'more') + '>' + I('more') + '</button></div>';
    var acc = '<div class="acc"><button class="lbl"' + A('sel') + '>' + I('select-all') + (S.sel ? 'Zrušit výběr' : 'Vybrat slovo') + '</button><button class="ib"><b>B</b></button><button class="ib"><i>I</i></button><button class="ib">' + I('list') + '</button><button class="ib" aria-label="Vložit fotku"' + A('camera') + '>' + I('camera') + '</button>' +
      (S.kbd ? '<button class="ib" aria-label="Skrýt klávesnici"' + A('hidekbd') + '>' + I('down') + '</button>' : '') + '</div>';
    var outline = '<div class="pane side"><div class="ph-h">Osnova</div><div class="row on">Zpráva o projektu</div><div class="row">Shrnutí</div></div>';
    var agentPane = '<div class="pane side" style="flex-basis:220px;box-shadow:inset 3px 0 0 var(--c-ai)"><div class="ph-h" style="color:var(--c-ai-ink)">Agent</div><div style="padding:0 14px">Shrnul jsem dokument do jednoho odstavce.<div style="margin-top:8px"><button class="btn small">Vrátit změnu agenta</button></div></div></div>';
    var mid = '<div class="pane" style="display:flex;flex-direction:column">' + docBody() + '</div>';
    return bar + '<div class="panes">' + (g.cls !== 'compact' ? outline : '') + mid + (g.cls === 'expanded' ? agentPane : '') + '</div>' + acc;
  }
  function overview() {
    var cards = [['editor', 'Editor', 'zprava.md'], ['files', 'Soubory', 'Dokumenty'], ['vault', 'Trezor hesel', '', 1]];
    return '<div class="ovw">' + cards.map(function (c) {
      return '<div class="ovc"' + (c[3] ? '' : A('card', c[0])) + '><div class="hdr">' + I(c[0] === 'vault' ? 'vault' : APPS[c[0]][0]) + c[1] + '</div><div class="shot' + (c[3] ? ' priv' : '') + '">' +
        (c[3] ? '<div style="text-align:center">' + I('lock') + '<br>Obsah skrytý<br><small>citlivá aplikace</small></div>' : '<div class="ln" style="width:50%"></div><div class="ln"></div><div class="ln" style="width:80%"></div><div class="ln" style="width:65%"></div>') + '</div></div>';
    }).join('') + '</div><div style="display:flex;justify-content:center;padding:10px;background:color-mix(in srgb, var(--c-bg) 60%, transparent)"><button class="btn small">Zavřít vše</button></div>';
  }

  // ---- Prekryti ----
  var BAND = '<div class="sys-band">' + I('shieldcheck', 's') + 'Systém<span class="phrase">' + I('key', 's') + 'tvá fráze: <b>modrá konvice</b></span></div>';
  function sheet() {
    var s = S.sheet, h = '';
    if (s === 'perm') h = '<div class="sheet trusted">' + BAND + '<div class="in"><div class="who">' + I('app-editor') + '<div><b>Editor</b><small>MagnumOpus &middot; ověřený podpis</small></div></div><h3>Editor chce použít kameru</h3><p>Pro vložení fotky do zprava.md. Kontrolka v horní liště bude svítit.</p>' +
      '<div class="acts"><button class="btn"' + A('close') + '>Nepovolit</button><button class="btn"' + A('allow') + '>Jen teď</button><button class="btn primary"' + A('allow') + '>Povolit vždy</button></div><p style="margin-top:10px;font-size:.85em">Pravost ověříte dvojím stiskem tlačítka napájení.</p></div></div>';
    if (s === 'ident') h = '<div class="sheet trusted">' + BAND + '<div class="in"><h3>Na obrazovce je</h3><div class="who">' + I(S.scr === 'files' ? 'folder' : 'app-editor') + '<div><b>' + (S.scr === 'files' ? 'Soubory' : S.scr === 'editor' ? 'Editor' : 'Domovská obrazovka') + '</b><small>MagnumOpus &middot; ověřený podpis</small></div></div>' +
      '<div class="kv"><span>Soubory</span><span>jen vybrané (zprava.md)</span></div><div class="kv"><span>Kamera</span><span>' + (S.cam ? 'povoleno jen teď' : 'nepovoleno') + '</span></div><div class="kv"><span>Agent</span><span>smí číst zprava.md</span></div><div class="kv"><span>Vstup</span><span>fyzický dotyk</span></div>' +
      '<div class="acts"><button class="btn primary"' + A('close') + '>Zavřít</button></div></div></div>';
    if (s === 'agent') h = '<div class="sheet agent"><div class="in"><h3 style="display:flex;gap:8px;align-items:center;color:var(--c-ai-ink)">' + I('agent') + 'Agent</h3><p>Pracuje za vás, ale vždy odděleně: jeho změny jsou podtržené a dají se vrátit.</p>' +
      '<div class="kv"><span>Teď</span><span>Shrnuje zprava.md (2/3)</span></div><div class="kv"><span>Smí</span><span>číst zprava.md, psát do odstavce Shrnutí</span></div><div class="kv"><span>Nesmí</span><span>síť, jiné soubory, odesílat</span></div>' +
      '<div class="progress agent" style="margin:12px 0"><i style="width:66%"></i></div><div class="acts"><button class="btn">Pozastavit</button><button class="btn"' + A('close') + '>Zavřít</button></div></div></div>';
    if (s === 'more') h = '<div class="sheet"><div class="in">' + [['camera', 'Vložit fotku', 'camera'], ['history', 'Historie verzí', 'hist'], ['share', 'Sdílet&hellip;', 'close'], ['agent', 'Zeptat se agenta', 'agent'], ['multi-monitor', 'Zobrazit na monitoru', 'connect'], ['info', 'Kdo je na obrazovce', 'ident']].map(function (m) {
      return '<div class="row"' + (m[2] === 'camera' ? A('camera') : m[2] === 'close' ? A('close') : A('sheet', m[2])) + '>' + I(m[0]) + m[1] + '</div>'; }).join('') + '</div></div>';
    if (s === 'hist') h = '<div class="sheet"><div class="in"><h3>Historie verzí: zprava.md</h3><p>Každé uložení je verze. Obnova vytvoří novou verzi, nic se nepřepíše.</p>' +
      [['Teď', 'Editor, vy', 1], ['14:02', 'Terminál'], ['13:40', 'Agent: Shrnutí'], ['včera 17:15', 'Editor, vy']].map(function (v) { return '<div class="kv"><span>' + v[0] + ' &middot; ' + v[1] + '</span><span>' + (v[2] ? 'aktuální' : '<a href="#"' + A('close') + '>Obnovit</a>') + '</span></div>'; }).join('') +
      '<div class="acts"><button class="btn"' + A('close') + '>Zavřít</button></div></div></div>';
    if (s === 'connect') h = '<div class="sheet"><div class="in"><h3>Připojen monitor DELL U2723</h3><p>Dokud nevyberete, na monitoru se nic nezobrazí.</p>' +
      [['copy', 'Zrcadlit', 'stejný obraz; oznámení se skryjí'], ['multi-monitor', 'Rozšířit', 'další plocha pro okna'], ['screen-share', 'Doprovodné ovládání', 'Editor to zatím neumí', 1]].map(function (m) { return '<div class="row"' + (m[3] ? ' style="opacity:.5"' : A('close')) + '>' + I(m[0]) + '<div>' + m[1] + '<small>' + m[2] + '</small></div></div>'; }).join('') +
      '<div class="acts"><button class="btn"' + A('close') + '>Teď ne</button></div></div></div>';
    return h ? '<div class="scrim2"' + A('close') + '></div>' + h : '';
  }
  function shade() {
    var dark = document.documentElement.dataset.theme === 'dark';
    var locked = S.scr === 'lock';
    var T = [['wifi', 'Wi-Fi', 'Doma-5G', 1], ['bluetooth', 'Bluetooth', 'vypnuto'], ['mobile-data', 'Mobilní data', 'simulováno', 0, 1], ['bell-off', 'Nerušit', 'vypnuto'],
      ['moon', 'Tmavý motiv', dark ? 'zapnuto' : 'vypnuto', dark, 0, 'theme'], ['restart', 'Otáčení', 'automaticky', 1], ['power-profile', 'Spořič', '82 %'], ['multi-monitor', 'Monitor', 'připojit', 0, 0, 'connect']];
    return '<div class="scrim2"' + A('close') + '></div><div class="shade"><div style="display:flex;justify-content:space-between;font-weight:600;padding:6px 6px 0"><span>14:05</span><span>čt 12. 11.</span></div>' +
      '<div class="tiles">' + T.map(function (t) { return '<button class="tile' + (t[3] ? ' on' : '') + (t[4] ? ' na' : '') + '"' + (t[5] === 'theme' ? A('theme') : t[5] === 'connect' ? A('sheet', 'connect') : '') + '>' + I(t[0]) + '<span>' + t[1] + '<small>' + t[2] + '</small></span></button>'; }).join('') + '</div>' +
      '<div class="slider"><i></i>' + I('sun') + '</div><div class="ph-h" style="padding-left:2px">Oznámení</div>' +
      // Na zamcene obrazovce jen aplikace a pocet, bez obsahu a bez Nastaveni.
      (locked ? '<div class="ncard" style="background:var(--c-surface)">' + I('copy') + '<div><b>Soubory</b><small>1 oznámení &middot; obsah po odemknutí</small></div></div>' +
        '<div class="ncard ai" style="background:var(--c-surface);margin-top:6px">' + I('agent') + '<div><b>Agent</b><small>1 oznámení &middot; obsah po odemknutí</small></div></div>' :
      '<div class="ncard" style="background:var(--c-surface)">' + I('copy') + '<div><b>Kopírování dokončeno</b><small>1 240 souborů do USB flash</small></div></div>' +
      '<div class="ncard ai" style="background:var(--c-surface);margin-top:6px"' + A('sheet', 'agent') + '>' + I('agent') + '<div><b>Agent</b><small>Shrnutí zprava.md je hotové &middot; Zobrazit</small></div></div>' +
      '<div style="display:flex;gap:8px;margin-top:10px"><button class="btn small"' + A('open', 'settings') + '>' + I('settings') + 'Nastavení</button><button class="btn small"' + A('sheet', 'ident') + '>' + I('shieldcheck') + 'Kdo je na obrazovce</button></div>') + '</div>';
  }
  function keyboard(g) {
    var pin = S.scr === 'lock' && S.pin >= 0;
    var st = 'height:' + (g.kh + g.ins.b) + 'px;bottom:0;left:' + g.ins.l + 'px;padding-bottom:' + (g.ins.b + 4) + 'px';
    if (pin) {
      return '<div class="vkb secure" style="' + st + '"><div class="sug"><span class="sec">' + I('shieldcheck') + 'Bezpečné zadání: bez návrhů a bez schránky</span></div>' +
        [['1', '2', '3'], ['4', '5', '6'], ['7', '8', '9'], ['', '0', 'x']].map(function (r) { return '<div class="kr">' + r.map(function (k) { return k === 'x' ? '<button class="w"' + A('pinx') + '>' + I('left') + '</button>' : k ? '<button style="max-width:none"' + A('pinkey') + '>' + k + '</button>' : '<span style="flex:1"></span>'; }).join('') + '</div>'; }).join('') + '</div>';
    }
    var last = (S.scr === 'search' ? S.q : S.typed).split(/\s/).pop(), sug;
    var m = last ? WORDS.filter(function (w) { return w.indexOf(last.toLowerCase()) === 0 && w !== last.toLowerCase(); }).slice(0, 3) : [];
    sug = (m.length ? m : ['a', 'je', 'se']).map(function (w) { return '<button' + A('sug', w) + '>' + w + '</button>'; }).join('');
    function row(keys) { return '<div class="kr">' + keys.map(function (k) {
      if (k === 'SH') return '<button class="w" aria-label="Velká písmena"' + A('shift') + (S.shift ? ' style="background:var(--c-accent-soft)"' : '') + '>' + I('up') + '</button>';
      if (k === 'BS') return '<button class="w" aria-label="Smazat"' + A('bs') + '>' + I('left') + '</button>';
      if (k === 'SP') return '<button class="sp"' + A('key', ' ') + '>Čeština</button>';
      if (k === 'EN') return '<button class="w ent" aria-label="Enter"' + A('enter') + '>' + I('check') + '</button>';
      if (k === '123') return '<button class="w">123</button>';
      if (k === 'GL') return '<button class="w" aria-label="Jazyk">' + I('language') + '</button>';
      var c = S.shift ? k.toUpperCase() : k; return '<button' + A('key', c) + '>' + c + '</button>';
    }).join('') + '</div>'; }
    return '<div class="vkb" style="' + st + '"><div class="sug">' + sug + '</div>' +
      // Na sirku se rada s diakritikou schova; znaky jsou pod dlouhym stiskem (e -> e s hackem atd.).
      (S.land && !g.d.tab ? '' : row('ě š č ř ž ý á í é'.split(' '))) + row('q w e r t z u i o p ú'.split(' ')) + row('a s d f g h j k l ů'.split(' ')) +
      row(['SH'].concat('y x c v b n m'.split(' '), ['BS'])) + row(['123', 'GL', ',', 'SP', '.', 'EN']) + '</div>';
  }
  function status(g) {
    var trusted = S.sheet === 'perm' || S.sheet === 'ident';
    return '<div class="stb" style="height:' + g.ins.t + 'px;padding-left:' + (g.ins.l + 22) + 'px"><span>14:05</span>' + (trusted ? '<span class="lockt">' + I('shieldcheck') + 'Systém</span>' : '') +
      '<span class="sp"></span>' + (S.agent ? '<span class="agentdot" title="Agent pracuje">' + I('agent') + '</span>' : '') + (S.cam ? '<span class="camdot" title="Kamera používána"></span>' : '') + I('wifi') + I('battery') + '82 %</div>';
  }
  function nav(g) {
    if (S.scr === 'lock' && S.pin >= 0) return '';
    if (S.nav === 'btn') return '<div class="navb" style="height:' + g.ins.b + 'px"><button aria-label="Zpět"' + A('g', 'back') + '>' + I('left') + '</button><button aria-label="Domů"' + A('g', 'home') + '>' + I('home') + '</button><button aria-label="Přehled"' + A('g', 'overview') + '>' + I('workspaces') + '</button></div>';
    return '<div class="navh" style="height:' + g.ins.b + 'px"' + A('g', 'home') + ' title="Domů (tah nahoru)"><i></i></div>';
  }
  function reach(g) {
    if (S.hand === 'none') return '';
    var x = S.hand === 'r' ? '100%' : '0%', r1 = Math.round(Math.min(g.W, g.H) * 0.78), r2 = Math.round(Math.min(g.W, g.H) * 1.2);
    return '<div class="reach" style="background:radial-gradient(circle at ' + x + ' 104%, rgb(60 170 110 / .28) 0 ' + r1 + 'px, rgb(230 170 40 / .25) ' + r1 + 'px ' + r2 + 'px, rgb(210 60 70 / .22) ' + r2 + 'px)"></div>';
  }

  // ---- Popisy ----
  var SAY = {
    lock: 'Zamčená obrazovka: oznámení ukazují jen aplikaci a počet, obsah až po odemknutí. Agent má vlastní barvu i na zámku. PIN se zadává bezpečnou klávesnicí.',
    home: 'Domov: vše důležité je dole na dosah palce (hledání, dok). Hledání je stejná brána jako paleta příkazů na počítači. Telefon a Zprávy jsou jen simulované.',
    search: 'Hledání: pište na klávesnici, výsledky jsou rozdělené na aplikace, soubory a akce. Je to stejný katalog jako paleta na počítači.',
    files: 'Soubory: na úzkém displeji seznam a klepnutí otevře soubor; na šířku seznam s náhledem a historií verzí; na tabletu i místa.',
    editor: 'Editor: klepněte do textu, vyjede klávesnice a zmenší se jen dostupná oblast. Odstavec od agenta je podtržený jeho barvou. Nástroje jsou v liště nad klávesnicí.',
    overview: 'Přehled aplikací: karty se dají listovat; citlivé aplikace (trezor) ukazují místo náhledu zámek. Klepnutím na kartu se vrátíte do aplikace se zachovaným stavem.'
  };
  var STEPS = [
    ['Zamčeno', function () { reset(); S.scr = 'lock'; }],
    ['Domů', function () { reset(); S.scr = 'home'; }],
    ['Spustit Soubory', function () { reset(); S.scr = 'files'; }],
    ['Otevřít soubor', function () { reset(); S.scr = 'editor'; S.from = 'files'; }],
    ['Upravit, klávesnice', function () { reset(); S.scr = 'editor'; S.from = 'files'; S.kbd = true; if (!S.typed) S.typed = 'Zítra pošlu '; }],
    ['Otočit na šířku', function () { S.scr = 'editor'; S.kbd = true; S.land = true; S.shade = false; S.sheet = null; }],
    ['Přehled a návrat', function () { S.land = false; S.kbd = false; S.sheet = null; S.scr = 'overview'; }],
    ['Dialog oprávnění', function () { reset(); S.scr = 'editor'; S.from = 'files'; S.sheet = 'perm'; }],
    ['Agent', function () { reset(); S.scr = 'editor'; S.sheet = 'agent'; }],
    ['Kdo je na obrazovce', function () { reset(); S.scr = 'editor'; S.sheet = 'ident'; }]
  ];
  var STEP_SAY = [null, null, null, 'Klepnutí na zprava.md otevře Editor. Soubor předal systémový výběr; Editor dostal jen tento soubor.', 'Pište na klávesnici (klepejte na klávesy). Návrhy slov jsou české. Zkuste „Vybrat slovo“: objeví se úchyty a textová nabídka.',
    'Na šířku je okno širší než 600 jednotek, takže rozložení přejde na medium (přibude osnova) nebo expanded (i panel agenta), viz kontext vpravo. Text, kurzor i klávesnice zůstanou; generace kontextu se zvýšila.', null,
    'Důvěryhodný list: pruh Systém s frází, ve stavovém řádku štítek. Bez titulku okna je systémovým místem stavový řádek a pravost ověří dvojí stisk napájení.',
    'Agent je vždy odlišený barvou a ikonou, ukazuje co smí a co nesmí, a dá se zastavit. Jeho ikona svítí ve stavovém řádku, dokud pracuje.',
    'Dvojí stisk napájení ukáže identitu aplikace v popředí. Aplikace tento list nemůže vyvolat ani napodobit.'];
  function reset() { S.kbd = false; S.sel = false; S.sheet = null; S.shade = false; S.pin = -1; S.land = false; }

  // ---- Vykresleni ----
  var lastKey = '';
  function render() {
    var g = geo();
    var key = [S.dev, S.land, S.big, S.nav, g.kbd, g.cls].join();
    if (key !== lastKey) { if (lastKey) S.gen++; lastKey = key; }
    var body = S.scr === 'lock' ? lock() : S.scr === 'home' ? home(g) : S.scr === 'search' ? search() : S.scr === 'files' ? files(g) : S.scr === 'editor' ? editor(g) : overview();
    var cut = g.d.tab ? '' : S.land ? '<span class="cut" style="left:14px;top:' + (g.H / 2 - 6) + 'px"></span>' : '<span class="cut" style="top:11px;left:' + (g.W / 2 - 6) + 'px"></span>';
    document.getElementById('devw').innerHTML = '<div class="dev' + (g.d.tab ? ' dtab' : '') + '" style="zoom:' + g.d.z + '"><div class="scr' + (S.big ? ' big' : '') + (S.areas ? ' areas' : '') + '" style="width:' + g.W + 'px;height:' + g.H + 'px">' +
      cut + status(g) + '<div class="area" style="left:' + g.ins.l + 'px;top:' + g.ins.t + 'px;width:' + g.aw + 'px;height:' + g.ah + 'px">' + body + '</div>' +
      (g.kbd ? keyboard(g) : '') + (S.shade ? shade() : sheet()) + nav(g) + reach(g) + '</div></div>';
    document.getElementById('ctx').textContent = ctxJson(g);
    var sx = STEP_SAY[S.step];
    document.getElementById('say').innerHTML = '<b>' + (S.step >= 0 ? (S.step + 1) + '. ' + STEPS[S.step][0] : 'Volně') + '</b><br>' + (sx || SAY[S.scr]);
    document.querySelectorAll('#steps button').forEach(function (b, i) { b.classList.toggle('on', i === S.step); });
  }
  function go(scr) { S.step = -1; S.scr = scr; S.sheet = null; S.shade = false; S.sel = false; if (scr !== 'editor') S.kbd = false; render(); }

  // ---- Gesta ----
  function gesture(n) {
    S.step = -1;
    if (n === 'shade') { S.shade = !S.shade; S.sheet = null; }
    else if (n === 'power2') { S.shade = false; S.sheet = 'ident'; }
    else if (n === 'home') { if (S.scr !== 'lock') { reset(); S.scr = 'home'; } }
    else if (n === 'overview') { if (S.scr !== 'lock') { reset(); S.scr = 'overview'; } }
    else if (n === 'back') {
      // Smlouva Zpet: 1. docasna vrstva, 2. navigace v aplikaci, 3. predchozi aplikace / Domu.
      if (S.shade) S.shade = false;
      else if (S.sheet) S.sheet = null;
      else if (S.sel) S.sel = false;
      else if (S.kbd && S.scr === 'editor') S.kbd = false;
      else if (S.scr === 'lock' && S.pin >= 0) S.pin = -1;
      else if (S.scr === 'editor') S.scr = S.from === 'files' ? 'files' : 'home';
      else if (S.scr !== 'lock' && S.scr !== 'home') S.scr = 'home';
    }
    render();
  }

  // ---- Udalosti ----
  function onClick(e) {
    var t = e.target.closest && e.target.closest('[data-a]');
    if (!t) return;
    var a = t.dataset.a, v = t.dataset.v;
    if (a !== 'kbd') e.stopPropagation();
    switch (a) {
      case 'unlock': go('home'); return;
      case 'pin': S.pin = 0; break;
      case 'pinkey': S.pin++; if (S.pin >= 4) { S.pin = -1; go('home'); return; } break;
      case 'pinx': S.pin = Math.max(0, S.pin - 1); break;
      case 'open': if (v === 'files') { S.from = 'home'; go('files'); } else if (v === 'editor') { S.from = 'home'; go('editor'); } else if (v === 'agent') { S.sheet = 'agent'; S.shade = false; } else return; break;
      case 'search': S.q = ''; go('search'); return;
      case 'res': if (v === 'zprava.md') { S.from = 'home'; go('editor'); } return;
      case 'file': S.fileSel = v; if (/\.md$/.test(v) && geo().cls === 'compact') { S.from = 'files'; go('editor'); return; } break;
      case 'openfile': S.from = 'files'; go('editor'); return;
      case 'kbd': if (S.scr === 'editor' && !S.sheet) { S.kbd = true; S.step = -1; } else return; break;
      case 'hidekbd': S.kbd = false; break;
      case 'sel': S.sel = !S.sel; break;
      case 'undo': S.typed = S.typed.replace(/\S*\s*$/, ''); break;
      case 'camera': S.sheet = 'perm'; S.kbd = false; break;
      case 'allow': S.cam = true; S.sheet = null; break;
      case 'sheet': S.sheet = v; S.shade = false; break;
      case 'close': S.sheet = null; S.shade = false; break;
      case 'card': go(v); return;
      case 'theme': document.documentElement.dataset.theme = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark'; break;
      case 'g': gesture(v); return;
      case 'shift': S.shift = !S.shift; break;
      case 'key': type(v); S.shift = false; break;
      case 'bs': if (S.scr === 'search') S.q = S.q.slice(0, -1); else S.typed = S.typed.slice(0, -1); break;
      case 'enter': if (S.scr === 'search') { if (norm(S.q).indexOf('zpr') === 0) { S.from = 'home'; go('editor'); return; } } else type(' '); break;
      case 'sug': var cur = S.scr === 'search' ? S.q : S.typed; cur = cur.replace(/\S*$/, v + ' '); if (S.scr === 'search') S.q = cur; else S.typed = cur; break;
      default: return;
    }
    if (a !== 'kbd') S.step = -1;
    render();
  }
  function type(c) { if (S.scr === 'search') S.q += c; else S.typed += c; }

  document.addEventListener('mo-ready', function () {
    document.getElementById('steps').innerHTML = STEPS.map(function (s, i) { return '<button><span class="n">' + (i + 1) + '</span>' + s[0] + '</button>'; }).join('');
    document.querySelectorAll('#steps button').forEach(function (b, i) { b.addEventListener('click', function () { S.step = i; STEPS[i][1](); render(); }); });
    function seg(id, list, cur, f) {
      var el = document.getElementById(id);
      el.innerHTML = list.map(function (x) { return '<button role="radio" data-value="' + x[0] + '" aria-checked="' + (x[0] === cur) + '">' + x[1] + '</button>'; }).join('');
      window.MO_SEG(el, f);
    }
    seg('c-dev', Object.keys(DEV).map(function (k) { return [k, DEV[k].n]; }), S.dev, function (v) { S.dev = v; render(); });
    seg('c-nav', [['gest', 'Gesta (výchozí)'], ['btn', 'Tlačítka (přístupnost)']], S.nav, function (v) { S.nav = v; render(); });
    seg('c-hand', [['none', 'Skrýt'], ['r', 'Pravá ruka'], ['l', 'Levá ruka']], S.hand, function (v) { S.hand = v; render(); });
    function tog(id, k) { var b = document.getElementById(id); b.addEventListener('click', function () { S[k] = !S[k]; b.setAttribute('aria-pressed', S[k]); render(); }); }
    document.getElementById('c-rot').addEventListener('click', function () { S.land = !S.land; render(); });
    tog('c-big', 'big'); tog('c-areas', 'areas');
    document.querySelectorAll('[data-g]').forEach(function (b) { b.addEventListener('click', function () { gesture(b.dataset.g); }); });
    document.getElementById('devw').addEventListener('click', onClick);
    render();
  });
})();
