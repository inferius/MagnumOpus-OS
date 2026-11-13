/* Simulator televizniho rezimu pro design/televize.html.
 * Fokus se ridi prostorovou navigaci nad prvky .f (nejblizsi prvek ve smeru sipky) a skupinami
 * s pameti (data-g): pri navratu do skupiny jde fokus na naposledy vybrany prvek. To je presne
 * model, ktery navrhujeme do knihovny komponent; tady je napsany v JS jen pro prototyp. */
(function () {
  'use strict';

  var PROF = { tata: ['Táta', '#ffb36b', 18], mama: ['Máma', '#9fd8ff', 18], ema: ['Ema', '#ffd36b', 7], host: ['Host', '#cfc6d8', 12] };
  var S = { scr: 'prof', prof: 'tata', inp: 'remote', safe: false, big: false, rm: false, dlg: null, osd: true, q: '', qsrc: '', pin: 0, toast: '', gen: 1, step: 0,
    foc: null, mem: {}, playing: 'more', paused: false, pos: 1260 };
  var tToast = 0, tAux = 0;

  function I(n, c) { return '<svg class="ic' + (c ? ' ' + c : '') + '" aria-hidden="true"><use href="#ic-' + n + '"/></svg>'; }
  function F(g, k, cls, html, extra) { return '<div class="f ' + (cls || '') + '" data-g="' + g + '" data-k="' + k + '"' + (extra || '') + '>' + html + '</div>'; }
  function grad(a, b) { return 'background:linear-gradient(135deg,' + a + ',' + b + ')'; }

  // Katalog: [klic, nazev, popis, hodnoceni, barva1, barva2, prubeh %]
  var MEDIA = [['more', 'Moře zblízka', 'Dokument &middot; 52 min', 0, '#0f4c6b', '#3fa3b8', 70], ['dovolena', 'Dovolená 2025', 'Rodinné video &middot; 18 min', 0, '#6b4a0f', '#e0a64b', 35],
    ['kuchyne', 'Kuchyně, díl 4', 'Seriál &middot; 30 min', 12, '#5b1f3f', '#d0627f', 50], ['noc', 'Noc ve městě', 'Film &middot; 1 h 48 min', 15, '#1b1f4f', '#5960c8', 10],
    ['les', 'Pohádky z lesa', 'Pohádka &middot; 12 min', 0, '#1f4f2a', '#69c27f', 60], ['kresleni', 'Kreslíme zvířata', 'Pro děti &middot; 15 min', 0, '#4f3b1f', '#f0b45a', 20]];
  var APPS = [['folder', 'Média'], ['image', 'Fotky'], ['music', 'Hudba'], ['gamepad', 'Hry'], ['app-browser', 'Prohlížeč'], ['agent', 'Agent'], ['settings', 'Nastavení']];
  function m(k) { return MEDIA.filter(function (x) { return x[0] === k; })[0]; }
  function rating() { return PROF[S.prof][2]; }
  function locked(x) { return x[3] > rating(); }

  // ---- Obrazovky ----
  function profiles() {
    return '<div class="prof"><h2>Kdo se dívá?</h2><div class="ps">' + Object.keys(PROF).map(function (k) {
      var p = PROF[k];
      return F('p', k, 'p', '<span class="av" style="background:' + p[1] + '">' + p[0][0] + '</span><b>' + p[0] + '</b><small>' + (k === 'ema' ? 'dětský profil &middot; do 7 let' : k === 'host' ? 'bez historie' : k === 'tata' ? 'PIN rodiče' : '&nbsp;') + '</small>');
    }).join('') + '</div></div>';
  }
  function card(x) {
    var lk = locked(x);
    return F('cont', x[0], 'tc', (x[3] ? '<span class="rt">' + x[3] + '+</span>' : '') +
      (lk ? '<span class="lk">' + I('lock') + 'zamčeno</span>' : '') + x[1] + '<small>' + x[2] + '</small><span class="pr"><i style="width:' + x[6] + '%"></i></span>', ' style="' + grad(x[4], x[5]) + (lk ? ';filter:saturate(.35)' : '') + '"');
  }
  function home() {
    var kid = S.prof === 'ema', p = PROF[S.prof];
    var cont = (kid ? ['les', 'kresleni', 'noc'] : ['more', 'dovolena', 'kuchyne', 'noc']).map(m);
    var apps = APPS.filter(function (a) { return !kid || ['Média', 'Fotky', 'Hudba', 'Hry'].indexOf(a[1]) >= 0; });
    var ph = [['#4b6b8f', '#b9d3ec', 'Hory, srpen'], ['#8f5b4b', '#ecc4b9', 'Narozeniny'], ['#4b8f6b', '#b9ecd0', 'Zahrada'], ['#6b4b8f', '#d3b9ec', 'Zima'], ['#8f8a4b', '#ece6b9', 'Výlet']];
    var mu = [['#2a2a2a', '#7a7a7a', 'Klavírní večer'], ['#5a1f1f', '#c86060', 'Rádio Jazz'], ['#1f3f5a', '#60a0c8', 'Na dobrou noc'], ['#3f5a1f', '#a0c860', 'Do auta']];
    var row = function (id, title, sub, items) { return '<div class="rw" data-row="' + id + '"><h3>' + title + (sub ? ' <small>' + sub + '</small>' : '') + '</h3><div class="strip">' + items + '</div></div>'; };
    var top = '<div class="top">' + F('top', 'home', 'tb cur', I('home') + 'Domů') + F('top', 'search', 'tb', I('search') + 'Hledat') + (kid ? '' : F('top', 'set', 'tb', I('settings') + 'Nastavení')) +
      '<span class="sp"></span><span class="clk">20:15</span>' + F('top', 'prof', 'av', p[0][0], ' style="background:' + p[1] + '" aria-label="Profil ' + p[0] + '"') + '</div>';
    return '<div class="safe">' + top + '<div class="rows"><div class="rowsin">' +
      row('cont', 'Pokračovat ve sledování', kid ? 'jen do 7 let' : '', cont.map(card).join('')) +
      row('apps', 'Aplikace', '', apps.map(function (a) { return F('apps', a[1], 'tc app', I(a[0]) + a[1]); }).join('')) +
      row('foto', 'Fotky', 'naposledy přidané', ph.map(function (x, i) { return F('foto', 'f' + i, 'tc ph', x[2], ' style="' + grad(x[0], x[1]) + '"'); }).join('')) +
      row('hud', 'Hudba', '', mu.map(function (x, i) { return F('hud', 'h' + i, 'tc ph', x[2], ' style="' + grad(x[0], x[1]) + '"'); }).join('')) +
      '</div></div></div>';
  }
  function fmt(s) { var h = Math.floor(s / 3600), mi = Math.floor(s / 60) % 60, se = s % 60; return (h ? h + ':' + (mi < 10 ? '0' : '') : '') + mi + ':' + (se < 10 ? '0' : '') + se; }
  function player() {
    var x = m(S.playing), len = 52 * 60, pct = Math.min(100, S.pos / len * 100);
    var osd = S.osd ? '<div class="osd"><h2>' + x[1] + '</h2><div class="meta">' + x[2] + ' &middot; titulky: čeština &middot; zvuk: původní</div>' +
      '<div class="seek"><span>' + fmt(S.pos) + '</span><span class="sbar"><i style="width:' + pct + '%"></i><b style="left:' + pct + '%"></b></span><span>' + fmt(len) + '</span></div>' +
      '<div class="ctr">' + F('ctl', 'rew', '', '&minus;10 s') + F('ctl', 'pp', '', I(S.paused ? 'play' : 'pause') + (S.paused ? 'Přehrát' : 'Pozastavit')) + F('ctl', 'fwd', '', '+10 s') +
      '<span class="sp"></span>' + F('ctl', 'sub', '', 'Titulky') + F('ctl', 'aud', '', I('volume') + 'Zvuk') + F('ctl', 'info', '', I('info') + 'Informace') + '</div></div>' : '';
    return '<div class="vid"><div class="scene"></div></div><div class="sub"' + (S.osd ? ' style="bottom:200px"' : '') + '><span>Korálové útesy pokrývají méně než procento dna.</span></div>' + osd;
  }
  var KEYS = 'abcdefghijklmnopqrstuvwxyzěščřžýáíé'.split('');
  function search() {
    var q = S.q, res = MEDIA.filter(function (x) { return !q || x[1].toLowerCase().indexOf(q) >= 0 || x[2].toLowerCase().indexOf(q) >= 0; }).filter(function (x) { return S.prof !== 'ema' || !locked(x); });
    return '<div class="safe"><div class="srch"><div><div class="qf">' + I('search') + '<span>' + (q || '<span style="opacity:.5">Hledat</span>') + '</span><span class="caret"></span>' +
      (S.qsrc ? '<small style="margin-left:auto;opacity:.6;font-size:.6em">' + S.qsrc + '</small>' : '') + '</div>' +
      '<div class="kg">' + KEYS.map(function (c) { return F('kb', c, '', c); }).join('') + F('kb', 'sp', 'wide', 'mezera') + F('kb', 'del', 'wide', 'smazat') + '</div>' +
      '<div class="alt">' + F('alt', 'mic', '', I('mic') + 'Hlasem') + F('alt', 'phone', '', I('screen-share') + 'Z telefonu') + '</div></div>' +
      '<div class="res"><h3>' + (q ? 'Výsledky pro „' + q + '“ (' + res.length + ')' : 'Oblíbené') + '</h3><div class="rgrid">' + res.slice(0, 6).map(function (x) {
        return F('res', x[0], 'tc', x[1] + '<small>' + x[2] + '</small>', ' style="' + grad(x[4], x[5]) + '"');
      }).join('') + '</div></div></div></div>';
  }

  // ---- Dialogy (vrstva nad obrazovkou; fokus jen uvnitr) ----
  var BAND = '<div class="band">' + I('shieldcheck') + 'Systém<span>' + I('key') + 'fráze: <b>modrá konvice</b></span></div>';
  function dialog() {
    if (S.dlg === 'pin') {
      return '<div class="scrim"><div class="tdlg tr" role="alertdialog">' + BAND + '<h2>Odemknout „Noc ve městě“ (15+)</h2><p>Zadejte PIN rodiče. Jen tlačítky ovladače u televize; z telefonu ani hlasem to nejde.</p>' +
        '<div class="pin">' + [0, 1, 2, 3].map(function (i) { return '<i class="' + (i < S.pin ? 'on' : '') + '"></i>'; }).join('') + '</div>' +
        '<div class="pad">' + ['1', '2', '3', '4', '5', '6', '7', '8', '9', 'x', '0', 'del'].map(function (k) { return F('pad', k, '', k === 'x' ? 'Zrušit' : k === 'del' ? '&#9003;' : k); }).join('') + '</div></div></div>';
    }
    if (S.dlg === 'who') {
      var p = PROF[S.prof];
      return '<div class="scrim"><div class="tdlg tr" role="alertdialog">' + BAND + '<h2>Kdo je na obrazovce</h2><p>Vyvoláno podržením Domů na spárovaném ovladači. Aplikace tento panel nemůže nakreslit ani zachytit.</p>' +
        '<div class="kv"><span>Profil</span><span>' + p[0] + (S.prof === 'ema' ? ' (dětský, do 7 let)' : '') + '</span></div>' +
        '<div class="kv"><span>Aplikace v popředí</span><span>' + (S.scr === 'play' ? 'Média &middot; podepsáno systémem' : 'Domovská obrazovka') + '</span></div>' +
        '<div class="kv"><span>Mikrofon</span><span>vypnutý (jen po stisku)</span></div>' +
        '<div class="kv"><span>Ovládání z telefonu</span><span>' + (S.qsrc ? 'připojený telefon „Telefon Táta“' : 'žádné') + '</span></div>' +
        '<div class="tbtn">' + F('who', 'ok', '', 'Zavřít') + F('who', 'prof', '', 'Změnit profil') + '</div></div></div>';
    }
    if (S.dlg === 'mic') return '<div class="mic"><i></i>Poslouchám&hellip; <span style="opacity:.6;font-size:.8em">mluvte, pak pusťte tlačítko</span></div>';
    if (S.dlg === 'phone') return '<div class="mic">' + I('screen-share') + 'Na telefonu se otevřelo pole pro psaní&hellip;</div>';
    return '';
  }

  // ---- Fokus: prostorova navigace se skupinami a pameti ----
  function root() { return document.querySelector('#tvw .scr'); }
  function layer() { var r = root(), d = r.querySelector('.scrim'); return d || r; }
  function items() { return Array.prototype.slice.call(layer().querySelectorAll('.f')); }
  function find(g, k) { return items().filter(function (e) { return e.dataset.g === g && e.dataset.k === k; })[0]; }
  function setFoc(el) {
    items().forEach(function (e) { e.classList.toggle('on', e === el); });
    if (!el) return;
    S.foc = { g: el.dataset.g, k: el.dataset.k };
    S.mem[el.dataset.g] = el.dataset.k;
    scrollTo(el);
    info();
  }
  // Rada se posune tak, aby fokus zustal na druhe pozici; svisle tak, aby radek s fokusem byl nahore.
  function scrollTo(el) {
    var strip = el.closest('.strip'), rin = root().querySelector('.rowsin');
    if (strip) {
      var cards = Array.prototype.slice.call(strip.children), i = cards.indexOf(el), w = el.offsetWidth + 16;
      var vis = strip.parentNode.clientWidth, total = cards.length * w - 16;
      strip.style.transform = 'translateX(' + (-Math.max(0, Math.min(Math.max(0, i - 1) * w, total - vis + 8))) + 'px)';
    }
    if (rin) { var rw = el.closest('.rw'); rin.style.transform = 'translateY(' + (rw ? -rw.offsetTop : 0) + 'px)'; }
  }
  function center(r) { return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; }
  function move(dir) {
    var all = items(), cur = all.filter(function (e) { return e.classList.contains('on'); })[0];
    if (!cur) { setFoc(all[0]); return; }
    // Uvnitr rady (strip) se jde po poradi, i kdyz je dalsi karta mimo obraz.
    var strip = cur.closest('.strip');
    if (strip && (dir === 'l' || dir === 'r')) {
      var sib = dir === 'r' ? cur.nextElementSibling : cur.previousElementSibling;
      if (sib) setFoc(sib); else bump2();
      return;
    }
    var a = cur.getBoundingClientRect(), ca = center(a), best = null, bd = Infinity;
    all.forEach(function (e) {
      if (e === cur) return;
      var b = e.getBoundingClientRect(), cb = center(b), dx = cb.x - ca.x, dy = cb.y - ca.y;
      var ok = dir === 'u' ? b.bottom <= a.top + 4 : dir === 'd' ? b.top >= a.bottom - 4 : dir === 'l' ? b.right <= a.left + 4 : b.left >= a.right - 4;
      if (!ok) return;
      var main = (dir === 'u' || dir === 'd') ? Math.abs(dy) : Math.abs(dx), side = (dir === 'u' || dir === 'd') ? Math.abs(dx) : Math.abs(dy);
      var d = main + side * 2;
      if (d < bd) { bd = d; best = e; }
    });
    if (!best) { bump2(); return; }
    // Pamet skupiny: do jine skupiny se vracime na naposledy vybrany prvek.
    if (best.dataset.g !== cur.dataset.g && S.mem[best.dataset.g]) { var r = find(best.dataset.g, S.mem[best.dataset.g]); if (r) best = r; }
    setFoc(best);
  }
  // Konec rady: kratke zhoupnuti misto ticha (uzivatel vi, ze dal nic neni).
  function bump2() { var c = root().querySelector('.f.on'); if (!c || S.rm) return; c.animate([{ transform: 'scale(1.08) translateX(0)' }, { transform: 'scale(1.08) translateX(6px)' }, { transform: 'scale(1.08) translateX(0)' }], 160); }

  function toast(t) { S.toast = t; clearTimeout(tToast); var r = root(); var o = r.querySelector('.toast'); if (o) o.remove(); r.insertAdjacentHTML('beforeend', '<div class="toast">' + t + '</div>'); tToast = setTimeout(function () { var x = root().querySelector('.toast'); if (x) x.remove(); }, 1600); }

  // ---- Akce (semanticke; ovladac, gamepad i klavesnice je mapuji stejne) ----
  function ok() {
    var f = S.foc; if (!f) return;
    if (S.dlg === 'pin') {
      if (f.k === 'x') { S.dlg = null; S.pin = 0; return render(); }
      if (f.k === 'del') { S.pin = Math.max(0, S.pin - 1); return render(true); }
      S.pin++; if (S.pin >= 4) { S.dlg = null; S.pin = 0; S.playing = 'noc'; S.pos = 640; go('play'); toast('Odemčeno jednorázově, jen tento film'); } else render(true);
      return;
    }
    if (S.dlg === 'who') { S.dlg = null; if (f.k === 'prof') return go('prof'); return render(); }
    if (S.scr === 'prof') { S.prof = f.k; S.mem = {}; return go('home'); }
    if (S.scr === 'home') {
      if (f.g === 'top') { if (f.k === 'search') return go('search'); if (f.k === 'prof') return go('prof'); if (f.k === 'set') return toast('Simulováno: Nastavení'); return; }
      if (f.g === 'cont') { var x = m(f.k); if (locked(x)) { S.dlg = 'pin'; S.pin = 0; return render(); } S.playing = f.k; S.pos = Math.round(x[6] / 100 * 52 * 60); return go('play'); }
      return toast('Simulováno: ' + (f.g === 'apps' ? f.k : 'otevřít'));
    }
    if (S.scr === 'play') {
      if (!S.osd) { S.osd = true; return render(); }
      if (f.k === 'pp') { S.paused = !S.paused; return render(true); }
      if (f.k === 'rew' || f.k === 'fwd') { S.pos = Math.max(0, S.pos + (f.k === 'fwd' ? 10 : -10)); return render(true); }
      return toast('Simulováno: ' + { sub: 'volba titulků', aud: 'zvuková stopa', info: 'informace o pořadu' }[f.k]);
    }
    if (S.scr === 'search') {
      if (f.g === 'kb') { S.qsrc = ''; S.q = f.k === 'del' ? S.q.slice(0, -1) : S.q + (f.k === 'sp' ? ' ' : f.k); return render(true); }
      if (f.k === 'mic') return voice();
      if (f.k === 'phone') return fromPhone();
      if (f.g === 'res') { var y = m(f.k); if (locked(y)) { S.dlg = 'pin'; return render(); } S.playing = f.k; S.pos = 0; return go('play'); }
    }
  }
  function voice() { S.dlg = 'mic'; render(true); clearTimeout(tAux); tAux = setTimeout(function () { S.dlg = null; S.q = 'moře'; S.qsrc = 'hlasem'; bumpGen(); render(true); }, 1400); }
  function fromPhone() { S.dlg = 'phone'; render(true); clearTimeout(tAux); tAux = setTimeout(function () { S.dlg = null; S.q = 'kuchyně'; S.qsrc = 'z telefonu'; bumpGen(); render(true); }, 1400); }
  function back() {
    if (S.dlg) { S.dlg = null; S.pin = 0; return render(); }
    if (S.scr === 'play') { if (S.osd) { S.osd = false; return render(); } return go('home'); }
    if (S.scr === 'search') return go('home');
    if (S.scr === 'home') { var f = S.foc; if (f && f.g !== 'top') { S.mem.top = 'home'; setFoc(find('top', 'home')); return; } return toast('Jste doma; Zpět dál nevede'); }
    if (S.scr === 'prof') toast('Vyberte profil');
  }
  function key(k) {
    clearTimeout(tAux);
    if (S.dlg === 'mic' || S.dlg === 'phone') S.dlg = null;
    if (k === 'home') { S.dlg = null; return go(S.scr === 'prof' ? 'prof' : 'home'); }
    if (k === 'homeHold') { S.dlg = 'who'; return render(); }
    if (k === 'back') return back();
    if (k === 'ok') return ok();
    if (k === 'menu') return toast('Kontextová nabídka (podržet OK)');
    if (k === 'mic') { if (S.scr !== 'search') go('search'); return voice(); }
    if (k === 'play') { if (S.scr === 'play') { S.paused = !S.paused; S.osd = true; return render(true); } return toast('Nic nehraje'); }
    if (k === 'vol+' || k === 'vol-') return toast('Hlasitost ' + (k === 'vol+' ? '42' : '38') + ' % (řeší systém, aplikace ji nedostane)');
    if (S.scr === 'play' && !S.osd && (k === 'l' || k === 'r')) { S.pos = Math.max(0, S.pos + (k === 'r' ? 10 : -10)); return toast((k === 'r' ? '+10 s' : '&minus;10 s') + ' &middot; ' + fmt(S.pos)); }
    move(k);
  }

  // ---- Vykresleni ----
  function bumpGen() { S.gen++; }
  function go(scr) { S.scr = scr; S.dlg = null; S.step = S.stepLock ? S.step : -1; if (scr === 'play') S.osd = true; if (scr === 'search' && !S.stepLock) { S.q = ''; S.qsrc = ''; } bumpGen(); render(); }
  function defFocus() {
    if (S.dlg === 'pin') return find('pad', '1');
    if (S.dlg === 'who') return find('who', 'ok');
    if (S.scr === 'prof') return find('p', S.prof);
    if (S.scr === 'home') return find('cont', S.mem.cont || items().filter(function (e) { return e.dataset.g === 'cont'; })[0].dataset.k);
    if (S.scr === 'play') return find('ctl', 'pp');
    if (S.scr === 'search') { var r = items().filter(function (e) { return e.dataset.g === 'res'; }); return S.q && r.length ? r[0] : find('kb', 'a'); }
  }
  // keep=true: zachovat fokus na stejnem prvku (prekresleni obsahu, ne zmena obrazovky).
  function render(keep) {
    var prev = S.foc;
    var body = S.scr === 'prof' ? profiles() : S.scr === 'home' ? home() : S.scr === 'play' ? player() : search();
    var host = document.getElementById('tvw'), z = window.innerWidth < 1250 ? 0.6 : 0.72;
    host.innerHTML = '<div class="tv" style="transform:scale(' + z + ')"><div class="scr' + (S.safe ? ' showsafe' : '') + (S.big ? ' big' : '') + (S.rm ? ' rm' : '') + '">' + body + dialog() + '</div></div>';
    host.style.width = Math.round(980 * z) + 'px'; host.style.height = Math.round(600 * z) + 'px';
    var el = null;
    if (keep && prev) el = find(prev.g, prev.k);
    if (!el && S.scr === 'play' && !S.osd) { S.foc = null; info(); return; }
    setFoc(el || defFocus());
  }
  function info() {
    var p = PROF[S.prof];
    var o = { gen: S.gen, app: S.scr === 'play' ? 'media' : 'shell', size: { w: 960, h: 540 }, scale: 2, 'class': 'expanded', posture: 'tv', viewing_distance: 'room',
      safe: { top: 27, right: 48, bottom: 27, left: 48 }, pointers: [], keys: S.inp === 'remote' ? 'remote' : 'gamepad', input_source: S.qsrc === 'z telefonu' ? 'remote_device' : 'local_physical',
      profile: { id: S.prof, rating_max: p[2] }, obscured: S.dlg === 'mic' || S.dlg === 'phone' ? [{ kind: 'system_panel', h: 80 }] : [], text_scale: S.big ? 1.2 : 1, reduce_motion: S.rm };
    document.getElementById('ctx').textContent = '{\n' + Object.keys(o).map(function (k) { return '  "' + k + '": ' + JSON.stringify(o[k]).replace(/,"/g, ', "').replace(/":/g, '": '); }).join(',\n') + '\n}' +
      '\n\n// fokus (knihovna komponent, ne kontext):\n// skupina ' + (S.foc ? S.foc.g + ', prvek ' + S.foc.k : '-') + '\n// paměť skupin: ' + JSON.stringify(S.mem).replace(/,"/g, ', "');
    var st = STEPS[S.step];
    document.getElementById('say').innerHTML = '<b>' + (S.step >= 0 ? (S.step + 1) + '. ' + st[0] : 'Volně') + '</b><br>' + (S.step >= 0 ? st[1] : 'Ovládejte ovladačem nebo šipkami, Enter = OK, Backspace = Zpět, H = Domů (podržet: G).');
    document.querySelectorAll('#steps button').forEach(function (b, i) { b.classList.toggle('on', i === S.step); });
  }

  // ---- Ovladac / gamepad ----
  function remote() {
    var b = function (k, h, cls, lbl) { return '<button data-key="' + k + '"' + (cls ? ' class="' + cls + '"' : '') + ' aria-label="' + lbl + '">' + h + '</button>'; };
    var dpad = '<div class="dpad">' + b('u', '&#9650;', 'u', 'Nahoru') + b('d', '&#9660;', 'd', 'Dolů') + b('l', '&#9664;', 'l', 'Vlevo') + b('r', '&#9654;', 'r', 'Vpravo') + b('ok', S.inp === 'remote' ? 'OK' : 'A', 'ok', 'OK') + '</div>';
    if (S.inp === 'gamepad') {
      return '<div class="rem" style="border-radius:40px;width:170px">' + I('gamepad') + dpad + '<div class="r2">' + b('ok', 'A', '', 'A (OK)') + b('back', 'B', '', 'B (Zpět)') + '</div>' +
        '<div class="r2">' + b('menu', '&#9776;', '', 'Start (nabídka)') + b('home', I('home'), '', 'Guide (Domů)') + '</div>' + b('homeHold', 'podržet Guide', '', 'Podržet Guide') + '<div class="hint">A = OK, B = Zpět, Start = nabídka, Guide = Domů</div></div>';
    }
    return '<div class="rem">' + '<div class="r2">' + b('mic', I('mic'), '', 'Hlas') + b('menu', '&#9776;', '', 'Nabídka') + '</div>' + dpad +
      '<div class="r2">' + b('back', '&#8617;', '', 'Zpět') + b('home', I('home'), '', 'Domů') + '</div>' +
      '<div class="r2">' + b('play', I('play'), '', 'Přehrát / pozastavit') + '<div class="vol">' + b('vol+', '+', '', 'Hlasitost nahoru') + b('vol-', '&minus;', '', 'Hlasitost dolů') + '</div></div>' +
      b('homeHold', 'podržet Domů', '', 'Podržet Domů') + '<div class="hint">podržení Domů jde vždy jen systému</div></div>';
  }
  function remoteRender() { document.getElementById('rem').innerHTML = remote(); }

  // ---- Pruchod ----
  var STEPS = [
    ['Profily', 'Po zapnutí volba profilu. Bez hesla, aby to šlo ovladačem; Táta má PIN rodiče pro odemykání. Šipky vlevo a vpravo, OK.', function () { S.scr = 'prof'; S.prof = 'tata'; S.mem = {}; }],
    ['Domů', 'Řady: pokračovat ve sledování, aplikace, fotky, hudba. Fokus je na první kartě; zvětšení, dvojitý obrys a stín. Všechno je uvnitř bezpečného okraje.', function () { S.prof = 'tata'; S.scr = 'home'; S.mem = {}; }],
    ['Paměť fokusu', 'Fokus byl na třetí kartě, uživatel šel dolů do aplikací a zpět nahoru: vrátil se na třetí kartu, ne na první. Řada se posunula, aby fokus zůstal na druhém místě.', function () { S.prof = 'tata'; S.scr = 'home'; S.mem = { cont: 'kuchyne', apps: 'Hudba' }; S.focTo = ['cont', 'kuchyne']; }],
    ['Přehrávač', 'OK na kartě spustí přehrávání. Ovládací prvky jsou sémantické akce (přehrát, posun, titulky); stejné přijdou z ovladače, gamepadu, telefonu i hlasu. Zpět nejdřív schová ovládání, podruhé odejde.', function () { S.prof = 'tata'; S.scr = 'play'; S.playing = 'more'; S.pos = 2184; S.paused = false; S.osd = true; }],
    ['Psaní z telefonu', 'Klávesnice v mřížce je pomalá, proto „Z telefonu“: na spárovaném telefonu se otevře pole a text přijde jako jedno vložení. Kontext ukazuje zdroj vstupu <code>remote_device</code>.', function () { S.prof = 'tata'; S.scr = 'search'; S.q = 'kuchyně'; S.qsrc = 'z telefonu'; }],
    ['Hledání hlasem', 'Tlačítko mikrofonu na ovladači: poslouchá jen během stisku. Panel je pro aplikaci překážka v kontextu.', function () { S.prof = 'tata'; S.scr = 'search'; S.q = ''; S.qsrc = ''; S.dlgTo = 'mic'; }],
    ['Dětský profil', 'Ema (do 7 let): jen dětské aplikace a obsah. Film 15+ je vidět se zámkem a nižší sytostí, aby bylo jasné, proč nejde. Rodič může zvolit úplné skrytí.', function () { S.prof = 'ema'; S.scr = 'home'; S.qsrc = ''; S.mem = { cont: 'noc' }; S.focTo = ['cont', 'noc']; }],
    ['PIN rodiče', 'Důvěryhodný dialog s frází. PIN jen tlačítky spárovaného ovladače u televize: telefon ani hlas ho nezadají (vstup z jiného zdroje se zahodí). Odemkne jen tento film, jednou.', function () { S.prof = 'ema'; S.scr = 'home'; S.qsrc = ''; S.mem = { cont: 'noc' }; S.focTo = ['cont', 'noc']; S.dlg = 'pin'; S.pin = 1; }],
    ['Kdo je na obrazovce', 'Podržení Domů na spárovaném ovladači: bezpečné vyvolání pro televizi. Ukáže profil, aplikaci v popředí, stav mikrofonu a připojené telefony.', function () { S.prof = 'ema'; S.scr = 'home'; S.qsrc = ''; S.dlg = 'who'; }]
  ];
  function step(i) {
    clearTimeout(tAux);
    S.step = i; S.dlg = null; S.focTo = null; S.dlgTo = null; S.stepLock = true;
    STEPS[i][2]();
    S.gen++;
    render();
    if (S.focTo && !S.dlg) setFoc(find(S.focTo[0], S.focTo[1]));
    if (S.dlgTo === 'mic') { S.dlg = 'mic'; render(true); }
    S.stepLock = false;
  }

  document.addEventListener('mo-ready', function () {
    document.getElementById('steps').innerHTML = STEPS.map(function (s, i) { return '<button>' + (i + 1) + '. ' + s[0] + '</button>'; }).join('');
    document.querySelectorAll('#steps button').forEach(function (b, i) { b.addEventListener('click', function () { step(i); }); });
    var seg = document.getElementById('c-inp');
    seg.innerHTML = '<button role="radio" data-value="remote" aria-checked="true">Dálkový ovladač</button><button role="radio" data-value="gamepad" aria-checked="false">Gamepad</button>';
    MO_SEG(seg, function (v) { S.inp = v; remoteRender(); info(); });
    [['c-safe', 'safe'], ['c-big', 'big'], ['c-rm', 'rm']].forEach(function (p) {
      var b = document.getElementById(p[0]);
      b.addEventListener('click', function () { S[p[1]] = !S[p[1]]; b.setAttribute('aria-pressed', S[p[1]]); bumpGen(); render(true); });
    });
    document.getElementById('rem').addEventListener('click', function (e) { var b = e.target.closest('[data-key]'); if (!b) return; S.step = -1; key(b.dataset.key); });
    document.getElementById('tvw').addEventListener('keydown', function (e) {
      var k = { ArrowUp: 'u', ArrowDown: 'd', ArrowLeft: 'l', ArrowRight: 'r', Enter: 'ok', Backspace: 'back', Escape: 'back', h: 'home', g: 'homeHold', m: 'mic', ' ': 'play' }[e.key];
      if (!k) return; e.preventDefault(); S.step = -1; key(k);
    });
    window.addEventListener('resize', function () { render(true); });
    remoteRender();
    step(0);
  });
})();
