// Knihovna komponent (design/komponenty.html): radky stavu a zive chovani.
// Klavesnice podle WAI-ARIA vzoru; kde se system lisi, rika to specifikace na strance.
(function () {
  'use strict';
  function $(s, r) { return (r || document).querySelector(s); }
  function $$(s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); }
  function I(n, c) { return '<svg class="ic ' + (c || '') + '" aria-hidden="true"><use href="#ic-' + n + '"/></svg>'; }
  function log(t) { var l = $('#klog'); if (l) l.textContent = t; }
  var root = document.documentElement;
  function dens() { return root.getAttribute('data-density'); }

  // ================= Radky stavu =================
  var STATES = [['def', 'Výchozí'], ['hover', 'Najetí'], ['press', 'Stisk'], ['focus', 'Fokus (klávesnice)'],
    ['dis', 'Vypnuto'], ['err', 'Chyba'], ['sel', 'Vybráno']];
  var CL = { hover: ' is-hover', press: ' is-press', focus: ' is-focus', err: ' err' };
  function c(s) { return CL[s] || ''; }
  function dis(s) { return s === 'dis'; }

  function btn(kind, label) {
    return function (s) {
      if (s === 'err' || s === 'sel') return null;
      if (s === 'busy') return '<button class="k-btn ' + kind + ' busy"><span class="k-spin"></span>Ukládám</button>';
      return '<button class="k-btn ' + kind + ' k-f' + c(s) + '"' + (dis(s) ? ' disabled' : '') + '>' + label + '</button>';
    };
  }
  function field(inner, s, extra) {
    if (s === 'press' || s === 'sel') return null;
    var h = '<div class="k-in' + (extra || '') + c(s) + (dis(s) ? ' k-dis' : '') + '">' + inner.replace(/<(input|textarea)/, '<$1' + (dis(s) ? ' disabled' : '')) + '</div>';
    if (s === 'err') h += '<span class="k-msg err">' + I('error') + '<span>Opravte hodnotu</span></span>';
    return h;
  }
  function chk(role, cls, label, inner) {
    return function (s) {
      var on = s === 'sel' ? 'true' : s === 'mix' ? 'mixed' : 'false';
      return '<span class="k-chk ' + cls + c(s) + (dis(s) ? ' k-dis' : '') + '" role="' + role + '" aria-checked="' + on + '">' + inner + label + '</span>';
    };
  }
  var BOX = '<span class="box">' + I('check', 'y') + '</span>';
  var T = {
    'btn-pri': btn('pri', 'Uložit'),
    'btn-sec': btn('', 'Zrušit'),
    'btn-dng': btn('dng', 'Smazat'),
    'btn-ico': function (s) {
      if (s === 'err') return null;
      return '<button class="k-btn ico k-f' + c(s) + '" aria-pressed="' + (s === 'sel') + '"' + (dis(s) ? ' disabled' : '') + '>' + I('pin') + '</button>';
    },
    'in-text': function (s) { return field('<input value="Dovolená 2026">', s); },
    'in-num': function (s) {
      return field('<input value="' + (s === 'err' ? '900' : '64') + '"><span class="unit">GiB</span><div class="k-step"><button>' + I('up') +
        '</button><button>' + I('down') + '</button></div>', s, ' num');
    },
    'in-pwd': function (s) { return field('<input type="password" value="heslo123"><button class="k-inb">' + I('eye') + '</button>', s); },
    'in-search': function (s) { return field(I('search', 's') + '<input value="faktura"><button class="k-inb">' + I('close', 's') + '</button>', s); },
    'in-area': function (s) { return field('<textarea rows="2">Poznámka</textarea>', s, ' area'); },
    chk: chk('checkbox', '', 'Fotky', BOX),
    sw: chk('switch', 'sw', 'Wi-Fi', '<span class="k-sw"></span>'),
    rad: chk('radio', 'rad', 'Uspat', '<span class="box"></span>'),
    sl: function (s) {
      if (s === 'sel') return null;
      return '<div class="k-sl' + c(s) + (dis(s) ? ' k-dis' : '') + '" style="width:120px"><div class="tr"></div><div class="fl" style="width:' + (s === 'err' ? 90 : 40) +
        '%"></div><div class="kn" style="left:' + (s === 'err' ? 90 : 40) + '%"></div></div>';
    },
    sel: function (s) {
      if (s === 'press') return null;
      return '<button class="k-in k-sel' + c(s) + (dis(s) ? ' k-dis' : '') + '" style="min-width:0;width:100%" aria-expanded="' + (s === 'sel') + '"><span class="v">Čeština</span>' + I('down', 's chev') + '</button>' +
        (s === 'err' ? '<span class="k-msg err">' + I('error') + '<span>Vyberte jazyk</span></span>' : '');
    },
    tab: function (s) {
      return '<div class="k-tabs" style="width:auto"><button class="k-tab' + c(s) + '" aria-selected="' + (s === 'sel') + '"' + (dis(s) ? ' disabled' : '') + '>' +
        (s === 'err' ? '<span class="dot"></span>' : '') + 'Historie</button></div>';
    },
    seg: function (s) {
      if (s === 'press') s = 'hover';
      var a = s === 'sel';
      return '<div class="k-seg' + (s === 'err' ? ' err' : '') + '"><button class="' + c(s === 'err' ? '' : s).trim() + '" aria-checked="' + a + '"' + (dis(s) ? ' disabled' : '') +
        '>Mřížka</button><button aria-checked="' + !a + '">Seznam</button></div>';
    },
    mi: function (s) {
      var cls = s === 'hover' ? ' is-hover' : s === 'press' || s === 'focus' ? ' act' : '';
      var it = s === 'err' ? '<div class="k-mi dng">' + I('trash') + 'Smazat<span class="sc">Del</span></div>'
        : s === 'sel' ? '<div class="k-mi"><span class="chk">' + I('check', 's') + '</span>Automaticky ukládat</div>'
        : '<div class="k-mi' + cls + '"' + (dis(s) ? ' aria-disabled="true"' : '') + '>' + I('copy') + 'Kopírovat<span class="sc">Ctrl+C</span></div>';
      return '<div class="k-pop" style="position:static;animation:none;min-width:0;width:100%">' + it + '</div>';
    },
    li: function (s) {
      if (s === 'press') return null;
      return '<ul class="k-list" style="width:100%;padding:0;border:0;background:transparent"><li class="k-li' + c(s) + (dis(s) ? ' k-dis' : '') + '" aria-selected="' + (s === 'sel') + '">' +
        I(s === 'err' ? 'error' : 'folder') + 'Dokumenty</li></ul>';
    },
    tn: function (s) {
      if (s === 'press') return null;
      return '<ul class="k-tree" style="width:100%;padding:0;border:0;background:transparent"><li aria-expanded="' + (s === 'sel') + '"' + (s === 'sel' ? ' aria-selected="true"' : '') + '>' +
        '<div class="k-tn' + (s === 'hover' ? ' is-hover' : '') + (dis(s) ? ' k-dis' : '') + '" style="' + (s === 'focus' ? 'box-shadow:inset 0 0 0 var(--k-ring) var(--c-focus);' : '') +
        (s === 'hover' ? 'background:var(--c-surface-3);' : '') + (s === 'err' ? 'color:var(--c-danger)' : '') + '"><span class="tw">' + I('right', 's') + '</span>' +
        I(s === 'err' ? 'error' : 'folder', s === 'err' ? '' : 'f') + 'Práce</div></li></ul>';
    },
    tag: function (s) {
      return '<span class="k-tag k-f' + c(s) + (dis(s) ? ' k-dis' : '') + '" aria-pressed="' + (s === 'sel') + '">Obrázky<button class="x">' + I('close') + '</button></span>';
    },
    crumb: function (s) {
      if (s === 'press' || s === 'dis' || s === 'err') return null;
      return '<nav class="k-crumbs"><ol><li>' + (s === 'sel' ? '<span aria-current="page">Faktury</span>' : '<a href="#drobecky" class="' + c(s).trim() + '">2026</a>') + '</li></ol></nav>';
    },
    acc: function (s) {
      if (s === 'press') return null;
      return '<div class="k-acc' + (s === 'err' ? ' err' : '') + (dis(s) ? ' k-dis' : '') + '"><button class="' + c(s === 'err' ? '' : s).trim() + '" aria-expanded="' + (s === 'sel') + '">' +
        (s === 'err' ? I('error', 's') : '') + 'Paměť' + I('down', 's chev') + '</button></div>';
    },
    toast: function (s) {
      var m = { info: ['info', 'Odkaz zkopírován'], ok: ['check', 'Uloženo'], warn: ['alert', 'Baterie 10 %'], dng: ['error', 'Nelze se připojit'] }[s];
      var act = s === 'dng' ? '<button class="k-btn">Zkusit znovu</button>' : s === 'info' ? '<button class="k-btn">Zpět</button>' : '';
      return '<div class="k-toast static ' + s + '"><span class="lead-ic">' + I(m[0], 's') + '</span><span class="t">' + m[1] + '</span>' + act + '</div>';
    },
    prog: function (s) {
      if (s === 'spin') return '<div class="k-spin"></div>';
      if (s === 'skel') return '<div class="k-pl" style="width:100%"><div class="k-skel" style="width:70%"></div><div class="k-skel"></div></div>';
      var w = { det: 60, ok: 100, err: 45, agent: 35, ind: 30 }[s];
      return '<div class="k-prog ' + s + '" style="width:100%"><i style="width:' + w + '%"></i></div>';
    }
  };
  T['btn-pri'].x = [['busy', 'Probíhá']];
  T.chk.x = [['mix', 'Neurčité']];
  T.toast.only = [['info', 'Informace + akce'], ['ok', 'Úspěch'], ['warn', 'Varování'], ['dng', 'Chyba + akce']];
  T.toast.wide = 1;
  T.mi.wide = 1;
  T.seg.wide = 1;
  T.prog.only = [['det', 'Určitý 60 %'], ['ind', 'Neurčitý'], ['ok', 'Hotovo'], ['err', 'Chyba'], ['agent', 'Agent'], ['spin', 'Kolečko'], ['skel', 'Kostra']];

  function strips() {
    $$('.k-states').forEach(function (box) {
      var t = T[box.dataset.k]; if (!t) return;
      var list = t.only || STATES.concat(t.x || []);
      if (t.wide) box.style.gridTemplateColumns = 'repeat(auto-fill, minmax(300px, 1fr))';
      box.innerHTML = list.map(function (p) {
        var h = t(p[0]);
        return '<div class="k-st' + (h == null ? ' na' : '') + '"><b>' + p[1] + '</b>' + (h || '') + '</div>';
      }).join('');
      box.inert = true;   // staticke ukazky: mimo poradi Tab a bez reakce
    });
  }

  // ================= Plovouci vrstva: umisteni, zasobnik =================
  function place(el, r, side) {
    var w = el.offsetWidth, h = el.offsetHeight, vw = document.documentElement.clientWidth, vh = window.innerHeight;
    var x = side === 'right' ? r.right - 4 : r.left, y = side === 'right' ? r.top - 6 : r.bottom + 6;
    if (x + w > vw - 8) x = side === 'right' ? r.left - w + 4 : vw - 8 - w;
    if (y + h > vh - 8) y = side === 'right' ? vh - 8 - h : r.top - 6 - h;
    el.style.left = Math.max(8, x) + window.scrollX + 'px';
    el.style.top = Math.max(8, y) + window.scrollY + 'px';
  }
  var stack = [];
  function closeTop(ret) {
    var p = stack.pop(); if (!p) return;
    p.el.remove(); if (p.onClose) p.onClose();
    if (ret !== false && p.opener && document.contains(p.opener)) p.opener.focus();
  }
  function closeAll(ret) { while (stack.length) closeTop(stack.length === 1 ? ret : false); }
  document.addEventListener('mousedown', function (e) {
    if (!stack.length) return;
    if (stack.some(function (p) { return p.el.contains(e.target); })) return;
    if (stack[0].opener && stack[0].opener.contains(e.target)) return;
    closeAll(false);
  }, true);
  window.addEventListener('resize', function () { closeAll(false); hideTip(); });

  // ================= Popisek =================
  var tip = null, tipT = 0;
  function hideTip() { clearTimeout(tipT); if (tip) { tip.remove(); tip = null; } }
  function showTip(el) {
    hideTip();
    if (el.getAttribute('aria-expanded') === 'true') return;
    tip = document.createElement('div'); tip.className = 'k-tip'; tip.setAttribute('role', 'tooltip');
    tip.innerHTML = el.dataset.tip + (el.dataset.key ? '<span class="kbd">' + el.dataset.key + '</span>' : '');
    document.body.appendChild(tip); place(tip, el.getBoundingClientRect(), 'below');
  }
  $$('[data-tip]').forEach(function (el) {
    el.addEventListener('mouseenter', function () {
      if (dens() !== 'desktop') return;
      clearTimeout(tipT); tipT = setTimeout(function () { showTip(el); }, 500);
    });
    el.addEventListener('mouseleave', hideTip);
    el.addEventListener('mousedown', hideTip);
    el.addEventListener('focus', function () {
      if (!el.matches(':focus-visible')) return;
      clearTimeout(tipT);
      if (dens() === 'tv') tipT = setTimeout(function () { showTip(el); }, 800); else showTip(el);
    });
    el.addEventListener('blur', hideTip);
  });

  // ================= Globalni klavesy =================
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      if (tip) { hideTip(); e.preventDefault(); return; }
      if (stack.length) { closeTop(); e.preventDefault(); return; }
    }
    if (e.key === 'F6') {
      var b = $('#toasts .k-toast button');
      if (b) { e.preventDefault(); b.focus(); }
    }
  });

  // ================= Hustota =================
  var DN = { desktop: 'myš a klávesnice, ovládací prvek 36 px', tablet: 'dotyk, ovládací prvek 44 px, bez najetí',
    tv: 'ovladač na 3 m, prvek 56 px, písmo 1,5 &times;, fokus 4 px' };
  window.MO_SEG($('#dens'), function (v) { root.setAttribute('data-density', v); $('#dens-note').innerHTML = DN[v]; closeAll(false); log('Hustota: ' + v); });

  $$('[data-log]').forEach(function (b) { b.addEventListener('click', function () { log('Klik: ' + b.dataset.log); }); });

  // ================= Tlacitka =================
  var pin = $('#b-pin');
  pin.addEventListener('click', function () {
    var v = pin.getAttribute('aria-pressed') !== 'true'; pin.setAttribute('aria-pressed', v); log(v ? 'Připnuto' : 'Odepnuto');
  });
  var busy = $('#b-busy');
  busy.addEventListener('click', function () {
    if (busy.classList.contains('busy')) { log('Ignorováno: akce už probíhá'); return; }
    var w = busy.offsetWidth; busy.style.width = w + 'px';
    busy.classList.add('busy'); busy.setAttribute('aria-busy', 'true'); busy.innerHTML = '<span class="k-spin"></span>';
    setTimeout(function () {
      busy.classList.remove('busy'); busy.removeAttribute('aria-busy'); busy.textContent = 'Odeslat'; busy.style.width = '';
      toast('ok', 'Odesláno');
    }, 1600);
  });
  $('#b-del').addEventListener('click', function (e) {
    dialog({ title: 'Smazat trvale 3 soubory?', body: 'Soubory nepůjdou obnovit z koše ani z historie.', opener: e.currentTarget,
      btns: [{ t: 'Zrušit', cancel: 1 }, { t: 'Smazat trvale', cls: 'dng', fn: function () { toast('dng', '3 soubory smazány'); } }] });
  });

  // ================= Pole =================
  var mail = $('#f-mail'), mailW = $('#f-mail-w'), mailE = $('#f-mail-e');
  function checkMail() {
    var ok = /^[^@\s]+@[^@\s]+\.[a-z]{2,}$/i.test(mail.value);
    mailW.classList.toggle('err', !ok); mail.setAttribute('aria-invalid', !ok); mailE.hidden = ok;
  }
  mail.addEventListener('blur', checkMail);
  mail.addEventListener('input', function () { if (mailW.classList.contains('err')) checkMail(); });
  checkMail();

  var num = $('#f-num'), numW = num.parentNode;
  function setNum(v, quiet) {
    var n = parseInt(v, 10), bad = isNaN(n) || n < 1 || n > 512;
    numW.classList.toggle('err', bad); num.setAttribute('aria-invalid', bad);
    if (!bad) { num.value = n; num.setAttribute('aria-valuenow', n); if (!quiet) log('Velikost: ' + n + ' GiB'); }
    else log('Chyba: mimo rozsah 1 až 512');
  }
  num.addEventListener('keydown', function (e) {
    var d = { ArrowUp: 1, ArrowDown: -1, PageUp: 10, PageDown: -10 }[e.key];
    if (e.key === 'Home') d = 1 - num.value; if (e.key === 'End') d = 512 - num.value;
    if (d) { e.preventDefault(); setNum(Math.min(512, Math.max(1, (parseInt(num.value, 10) || 0) + d))); }
  });
  num.addEventListener('input', function () { setNum(num.value, true); });
  $$('.k-step button', numW).forEach(function (b) {
    b.addEventListener('mousedown', function (e) { e.preventDefault(); num.focus(); });
    b.addEventListener('click', function () { setNum(Math.min(512, Math.max(1, (parseInt(num.value, 10) || 0) + +b.dataset.d))); });
  });

  var pwd = $('#f-pwd'), eye = $('#f-pwd-eye');
  eye.addEventListener('click', function () {
    var show = pwd.type === 'password'; pwd.type = show ? 'text' : 'password';
    eye.setAttribute('aria-pressed', show); eye.setAttribute('aria-label', show ? 'Skrýt heslo' : 'Zobrazit heslo');
    eye.dataset.tip = show ? 'Skrýt heslo' : 'Zobrazit heslo';
    eye.innerHTML = I(show ? 'eye-off' : 'eye');
  });
  ['keydown', 'keyup'].forEach(function (t) {
    pwd.addEventListener(t, function (e) { if (e.getModifierState) $('#f-caps').classList.toggle('on', e.getModifierState('CapsLock')); });
  });
  pwd.addEventListener('blur', function () { $('#f-caps').classList.remove('on'); });

  var q = $('#f-q'), qx = $('#f-q-x');
  function syncQ() { qx.hidden = !q.value; }
  q.addEventListener('input', syncQ);
  q.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    e.preventDefault(); e.stopPropagation();
    if (q.value) { q.value = ''; syncQ(); log('Hledání vymazáno'); } else { q.blur(); log('Pole hledání opuštěno'); }
  });
  qx.addEventListener('click', function () { q.value = ''; syncQ(); q.focus(); log('Hledání vymazáno'); });
  syncQ();

  var area = $('#f-area');
  function syncArea() { $('#f-area-c').textContent = area.value.length + ' / 500'; }
  area.addEventListener('input', syncArea);
  area.addEventListener('keydown', function (e) { if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); log('Ctrl+Enter: formulář odeslán'); } });
  syncArea();

  // ================= Volby =================
  function toggleable(el, onChange) {
    function flip() {
      if (el.getAttribute('aria-disabled') === 'true') return;
      var v = el.getAttribute('aria-checked') === 'true' ? 'false' : 'true';
      el.setAttribute('aria-checked', v); if (onChange) onChange(v);
      log(el.textContent.trim() + ': ' + (v === 'true' ? 'zapnuto' : 'vypnuto'));
    }
    el.addEventListener('click', flip);
    el.addEventListener('keydown', function (e) { if (e.key === ' ') { e.preventDefault(); flip(); } });
  }
  var ones = $$('.c-one'), all = $('#c-all');
  function syncAll() {
    var n = ones.filter(function (o) { return o.getAttribute('aria-checked') === 'true'; }).length;
    all.setAttribute('aria-checked', n === 0 ? 'false' : n === ones.length ? 'true' : 'mixed');
  }
  ones.forEach(function (o) { toggleable(o, syncAll); });
  toggleable(all, function (v) { ones.forEach(function (o) { o.setAttribute('aria-checked', v); }); });
  $$('.k-chk.sw:not(.k-dis)').forEach(function (s) { toggleable(s); });
  window.MO_SEG($('#r-grp'), function (v, b) { log('Po zavření víka: ' + b.textContent.trim()); });
  $$('#r-grp [role="radio"]').forEach(function (b) {
    b.addEventListener('keydown', function (e) { if (e.key === ' ') { e.preventDefault(); b.click(); } });
  });

  // ================= Posuvnik =================
  var sl = $('#sl');
  function setSl(v) {
    v = Math.max(0, Math.min(100, Math.round(v)));
    sl.setAttribute('aria-valuenow', v); sl.setAttribute('aria-valuetext', v + ' %');
    $('.fl', sl).style.width = v + '%'; $('.kn', sl).style.left = v + '%'; $('#sl-v').textContent = v + ' %';
    sl.classList.toggle('err', v > 85); log('Hlasitost ' + v + ' %' + (v > 85 ? ' (nad bezpečnou mez)' : ''));
  }
  sl.addEventListener('keydown', function (e) {
    var v = +sl.getAttribute('aria-valuenow');
    var d = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1, PageUp: 10, PageDown: -10 }[e.key];
    if (e.key === 'Home') d = -v; if (e.key === 'End') d = 100 - v;
    if (d !== undefined) { e.preventDefault(); setSl(v + d); }
  });
  function slAt(e) { var r = sl.getBoundingClientRect(); setSl((e.clientX - r.left) / r.width * 100); }
  sl.addEventListener('pointerdown', function (e) {
    sl.setPointerCapture(e.pointerId); slAt(e);
    function mv(ev) { slAt(ev); }
    function up() { sl.removeEventListener('pointermove', mv); sl.removeEventListener('pointerup', up); }
    sl.addEventListener('pointermove', mv); sl.addEventListener('pointerup', up);
  });
  sl.addEventListener('wheel', function (e) {
    if (document.activeElement !== sl) return;   // bez fokusu kolecko posouva stranku
    e.preventDefault(); setSl(+sl.getAttribute('aria-valuenow') + (e.deltaY < 0 ? 1 : -1));
  }, { passive: false });
  setSl(40);

  // ================= Vyber =================
  var LANGS = ['Čeština', 'Dansk', 'Deutsch', 'English (UK)', 'English (US)', 'Español', 'Français', 'Italiano', 'Magyar', 'Nederlands', 'Polski', 'Slovenčina', 'Suomi', 'Svenska'];
  var selB = $('#sel'), selV = 0, selPop = null, selAct = 0, typed = '', typedT = 0;
  function typeAhead(ch, list, from) {
    clearTimeout(typedT); typed += ch.toLowerCase(); typedT = setTimeout(function () { typed = ''; }, 1000);
    for (var k = 0; k < list.length; k++) {
      var i = (from + (typed.length === 1 ? 1 : 0) + k) % list.length;
      if (list[i].toLowerCase().indexOf(typed) === 0) return i;
    }
    return -1;
  }
  function selSet(i) { selV = i; $('.v', selB).textContent = LANGS[i]; log('Jazyk: ' + LANGS[i]); }
  function selHi(i) {
    selAct = i;
    $$('.k-opt', selPop).forEach(function (o, j) { o.classList.toggle('act', j === i); if (j === i) o.scrollIntoView({ block: 'nearest' }); });
    selB.setAttribute('aria-activedescendant', 'opt-' + i);
  }
  function selOpen() {
    hideTip();
    selPop = document.createElement('div'); selPop.className = 'k-pop'; selPop.setAttribute('role', 'listbox'); selPop.id = 'sel-lb';
    selPop.style.maxHeight = 'calc(var(--k-item) * 8 + 8px)'; selPop.style.overflowY = 'auto';
    selPop.style.minWidth = selB.offsetWidth + 'px';
    selPop.innerHTML = LANGS.map(function (l, i) {
      return '<div class="k-opt" role="option" id="opt-' + i + '" aria-selected="' + (i === selV) + '">' + l + (i === selV ? I('check', 's y') : '') + '</div>';
    }).join('');
    document.body.appendChild(selPop); place(selPop, selB.getBoundingClientRect(), 'below');
    selB.setAttribute('aria-expanded', 'true'); selB.setAttribute('aria-controls', 'sel-lb');
    stack.push({ el: selPop, opener: selB, onClose: function () { selPop = null; selB.setAttribute('aria-expanded', 'false'); selB.removeAttribute('aria-activedescendant'); } });
    $$('.k-opt', selPop).forEach(function (o, i) {
      o.addEventListener('mouseenter', function () { selHi(i); });
      o.addEventListener('click', function () { selSet(i); closeTop(); });
    });
    selHi(selV);
  }
  selB.addEventListener('click', function () { if (selPop) closeTop(); else selOpen(); });
  selB.addEventListener('keydown', function (e) {
    var k = e.key, n = LANGS.length;
    if (!selPop) {
      if (k === 'Enter' || k === ' ' || k === 'F4' || (k === 'ArrowDown' && e.altKey)) { e.preventDefault(); selOpen(); return; }
      if (k === 'ArrowDown' || k === 'ArrowUp') { e.preventDefault(); selSet(Math.max(0, Math.min(n - 1, selV + (k === 'ArrowDown' ? 1 : -1)))); return; }
      if (k.length === 1 && k !== ' ') { var j = typeAhead(k, LANGS, selV); if (j >= 0) selSet(j); }
      return;
    }
    if (k === 'ArrowDown') { e.preventDefault(); selHi(Math.min(n - 1, selAct + 1)); }
    else if (k === 'ArrowUp') { e.preventDefault(); selHi(Math.max(0, selAct - 1)); }
    else if (k === 'Home') { e.preventDefault(); selHi(0); }
    else if (k === 'End') { e.preventDefault(); selHi(n - 1); }
    else if (k === 'Enter' || k === ' ') { e.preventDefault(); selSet(selAct); closeTop(); }
    else if (k === 'Tab') { selSet(selAct); closeTop(false); }
    else if (k.length === 1) { var i = typeAhead(k, LANGS, selAct); if (i >= 0) selHi(i); }
  });

  // ================= Zalozky a segmenty =================
  var tabs = $$('#tabs [role="tab"]');
  function pickTab(t, focus) {
    tabs.forEach(function (x) { var on = x === t; x.setAttribute('aria-selected', on); x.tabIndex = on ? 0 : -1; });
    if (focus) t.focus();
    $('#tp').textContent = 'Obsah záložky ' + t.textContent.trim() + '.';
    log('Záložka: ' + t.textContent.trim());
  }
  tabs.forEach(function (t, i) {
    t.tabIndex = t.getAttribute('aria-selected') === 'true' ? 0 : -1;
    t.addEventListener('click', function () { pickTab(t); });
    t.addEventListener('keydown', function (e) {
      var en = tabs.filter(function (x) { return !x.disabled; }), j = en.indexOf(t), m = en.length;
      var to = { ArrowRight: en[(j + 1) % m], ArrowLeft: en[(j - 1 + m) % m], Home: en[0], End: en[m - 1] }[e.key];
      if (to) { e.preventDefault(); pickTab(to, true); }
    });
  });
  document.addEventListener('keydown', function (e) {
    if (!e.ctrlKey || !(e.key === 'PageDown' || e.key === 'PageUp')) return;
    if (!$('#zalozky').contains(document.activeElement)) return;
    var en = tabs.filter(function (x) { return !x.disabled; });
    var j = en.findIndex(function (x) { return x.getAttribute('aria-selected') === 'true'; });
    e.preventDefault(); pickTab(en[(j + (e.key === 'PageDown' ? 1 : -1) + en.length) % en.length], true);
  });
  window.MO_SEG($('#seg'), function (v, b) { log('Zobrazení: ' + b.textContent.trim()); });

  // ================= Nabidky =================
  function menu(items, rect, opener, side, onClose) {
    hideTip();
    var m = document.createElement('div'); m.className = 'k-pop'; m.setAttribute('role', 'menu');
    m.innerHTML = items.map(function (it, i) {
      if (it === '-') return '<div class="k-sep" role="separator"></div>';
      if (it.h) return '<div class="k-mh" role="presentation">' + it.h + '</div>';
      var role = it.chk !== undefined ? 'menuitemcheckbox' : 'menuitem';
      return '<div class="k-mi' + (it.dng ? ' dng' : '') + '" tabindex="-1" role="' + role + '" data-i="' + i + '"' +
        (it.chk !== undefined ? ' aria-checked="' + it.chk + '"' : '') + (it.dis ? ' aria-disabled="true" title="' + it.dis + '"' : '') +
        (it.sub ? ' aria-haspopup="menu" aria-expanded="false"' : '') + '>' +
        (it.chk !== undefined ? '<span class="chk">' + (it.chk ? I('check', 's') : '') + '</span>' : it.i ? I(it.i) : '<span class="chk"></span>') +
        '<span>' + it.t + '</span>' + (it.k ? '<span class="sc">' + it.k + '</span>' : '') + (it.sub ? I('right', 's sub') : '') + '</div>';
    }).join('');
    document.body.appendChild(m); place(m, rect, side);
    var entry = { el: m, opener: opener, onClose: onClose }; stack.push(entry);
    var mis = $$('.k-mi', m), cur = -1, subT = 0;
    function act(i) { cur = i; mis.forEach(function (x, j) { x.classList.toggle('act', j === i); }); if (i >= 0) mis[i].focus(); }
    function itemOf(el) { return items[+el.dataset.i]; }
    function openSub(el, focusFirst) {
      if (el.getAttribute('aria-expanded') === 'true') return;
      while (stack[stack.length - 1] !== entry) closeTop(false);
      el.setAttribute('aria-expanded', 'true');
      var sm = menu(itemOf(el).sub, el.getBoundingClientRect(), el, 'right', function () { el.setAttribute('aria-expanded', 'false'); });
      if (focusFirst) sm.act(0);
    }
    function run(el) {
      var it = itemOf(el);
      if (it.dis) { log('Nedostupné: ' + it.dis); return; }
      if (it.sub) { openSub(el, true); return; }
      if (it.chk !== undefined) {
        it.chk = !it.chk; el.setAttribute('aria-checked', it.chk); $('.chk', el).innerHTML = it.chk ? I('check', 's') : '';
        log(it.t + ': ' + (it.chk ? 'zapnuto' : 'vypnuto')); return;
      }
      closeAll(); log('Nabídka: ' + it.t);
      if (it.fn) it.fn();
    }
    mis.forEach(function (el, i) {
      el.addEventListener('mouseenter', function () {
        act(i); clearTimeout(subT);
        while (stack[stack.length - 1] !== entry && stack[stack.length - 1].opener !== el) closeTop(false);
        if (itemOf(el).sub) subT = setTimeout(function () { openSub(el, false); }, 300);
      });
      el.addEventListener('click', function () { run(el); });
    });
    m.addEventListener('keydown', function (e) {
      var k = e.key, n = mis.length;
      if (k === 'ArrowDown') { e.preventDefault(); e.stopPropagation(); act((cur + 1) % n); }
      else if (k === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); act((cur - 1 + n) % n); }
      else if (k === 'Home') { e.preventDefault(); act(0); }
      else if (k === 'End') { e.preventDefault(); act(n - 1); }
      else if (k === 'ArrowRight' && cur >= 0 && itemOf(mis[cur]).sub) { e.preventDefault(); e.stopPropagation(); openSub(mis[cur], true); }
      else if (k === 'ArrowLeft' && side === 'right') { e.preventDefault(); e.stopPropagation(); closeTop(); }
      else if (k === 'Enter' || k === ' ') { e.preventDefault(); e.stopPropagation(); if (cur >= 0) run(mis[cur]); }
      else if (k === 'Tab') { closeAll(); }
      else if (k.length === 1 && !e.ctrlKey) {
        var names = mis.map(function (x) { return itemOf(x).t; }), j = typeAhead(k, names, Math.max(cur, 0));
        if (j >= 0) act(j);
      }
    });
    return { act: act, el: m };
  }
  var FILE = [
    { t: 'Nový', i: 'file-plus', k: 'Ctrl+N' }, { t: 'Otevřít...', i: 'folder', k: 'Ctrl+O' },
    { t: 'Otevřít nedávné', i: 'history', sub: [{ t: 'Rozpočet 2026' }, { t: 'Faktura 0142' }, { t: 'Poznámky' }, '-', { t: 'Vymazat seznam' }] },
    '-', { t: 'Uložit', i: 'save', k: 'Ctrl+S' }, { t: 'Uložit jako...', dis: 'Soubor je jen pro čtení' }, { t: 'Automaticky ukládat', chk: true },
    '-', { t: 'Přesunout do koše', i: 'trash', k: 'Del', dng: 1, fn: function () { toast('info', 'Přesunuto do koše', 'Zpět'); } }
  ];
  var mb = $('#m-btn');
  function openFile(first) {
    mb.setAttribute('aria-expanded', 'true');
    var mm = menu(FILE, mb.getBoundingClientRect(), mb, 'below', function () { mb.setAttribute('aria-expanded', 'false'); });
    mm.act(first === 'last' ? $$('.k-mi', mm.el).length - 1 : 0);
  }
  mb.addEventListener('click', function () { if (mb.getAttribute('aria-expanded') === 'true') closeAll(); else openFile(); });
  mb.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); openFile(); }
    if (e.key === 'ArrowUp') { e.preventDefault(); openFile('last'); }
  });
  var CTX = [
    { h: 'Rozpočet.ods' }, { t: 'Otevřít', i: 'external', k: 'Enter' }, { t: 'Kopírovat', i: 'copy', k: 'Ctrl+C' },
    { t: 'Vložit', i: 'paste', k: 'Ctrl+V', dis: 'Schránka je prázdná' }, { t: 'Přejmenovat', i: 'rename', k: 'F2' },
    '-', { t: 'Sdílet', i: 'share', sub: [{ t: 'Odkazem' }, { t: 'E-mailem' }, { t: 'Do zařízení v síti' }] },
    '-', { t: 'Přesunout do koše', i: 'trash', k: 'Del', dng: 1, fn: function () { toast('info', 'Přesunuto do koše', 'Zpět'); } }
  ];
  var ctx = $('#ctx');
  function openCtx(x, y) {
    closeAll(false);
    var mm = menu(CTX, { left: x, right: x, top: y - 6, bottom: y - 6 }, ctx, 'below');
    mm.act(0);
  }
  ctx.addEventListener('contextmenu', function (e) { e.preventDefault(); openCtx(e.clientX, e.clientY); });
  ctx.addEventListener('keydown', function (e) {
    if ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu') {
      e.preventDefault(); var r = ctx.getBoundingClientRect(); openCtx(r.left + 16, r.top + 24);
    }
  });

  // ================= Bublina =================
  var pv = $('#pv-btn');
  pv.addEventListener('click', function () {
    if (pv.getAttribute('aria-expanded') === 'true') { closeAll(); return; }
    hideTip();
    var p = document.createElement('div'); p.className = 'k-pop pv'; p.setAttribute('role', 'dialog'); p.setAttribute('aria-label', 'Šifrování');
    p.innerHTML = '<span class="arrow"></span><h4>Šifrování disku</h4><p class="muted">Data jsou zašifrovaná klíčem uloženým v čipu TPM. Bez přihlášení je nikdo nepřečte, ani po vyjmutí disku.</p>' +
      '<div class="hstack"><a href="#bubliny" class="k-f">Více o šifrování</a><span class="spacer"></span><button class="k-btn pri k-f" style="height:var(--k-h-sm)">Rozumím</button></div>';
    document.body.appendChild(p); place(p, pv.getBoundingClientRect(), 'below');
    var r = pv.getBoundingClientRect(), pl = parseFloat(p.style.left) - window.scrollX;
    $('.arrow', p).style.left = Math.max(12, r.left + r.width / 2 - pl - 6) + 'px';
    pv.setAttribute('aria-expanded', 'true');
    stack.push({ el: p, opener: pv, onClose: function () { pv.setAttribute('aria-expanded', 'false'); } });
    $('.k-btn', p).addEventListener('click', function () { closeAll(); });
    $('a', p).focus();
  });

  // ================= Dialog =================
  function dialog(o) {
    hideTip(); closeAll(false);
    var s = document.createElement('div'); s.className = 'k-scrim';
    var d = document.createElement('div'); d.className = 'k-dlg'; d.setAttribute('role', 'dialog'); d.setAttribute('aria-modal', 'true');
    d.setAttribute('aria-labelledby', 'k-dlg-t'); d.setAttribute('aria-describedby', 'k-dlg-b');
    d.innerHTML = '<div class="hd"><h3 id="k-dlg-t">' + o.title + '</h3><button class="k-btn ico ghost k-f" data-x aria-label="Zavřít">' + I('close', 's') + '</button></div>' +
      '<div class="bd" id="k-dlg-b">' + o.body + '</div><div class="ft">' + o.btns.map(function (b, i) {
        return '<button class="k-btn k-f ' + (b.cls || '') + '" data-b="' + i + '">' + b.t + '</button>';
      }).join('') + '</div>';
    document.body.appendChild(s); document.body.appendChild(d);
    var opener = o.opener || document.activeElement;
    function close(i) {
      d.remove(); s.remove();
      if (opener) opener.focus();
      var b = o.btns[i];
      log('Dialog: ' + (b ? b.t : 'zavřeno'));
      if (b && b.fn) b.fn();
    }
    var cancel = o.btns.findIndex(function (b) { return b.cancel; });
    var def = o.btns.findIndex(function (b) { return b.cls === 'pri'; });
    $$('[data-b]', d).forEach(function (b) { b.addEventListener('click', function () { close(+b.dataset.b); }); });
    $('[data-x]', d).addEventListener('click', function () { close(cancel); });
    s.addEventListener('mousedown', function (e) { e.preventDefault(); });
    window.MO_TRAP(d, function () { close(cancel); });
    d.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' && def >= 0 && e.target.tagName !== 'BUTTON' && e.target.tagName !== 'TEXTAREA') { e.preventDefault(); close(def); }
      e.stopPropagation();
    });
    var first = $('input, textarea', d) || $('[data-b="' + (cancel >= 0 ? cancel : 0) + '"]', d);
    first.focus();
  }
  $('#d-open').addEventListener('click', function (e) {
    dialog({ title: 'Zahodit změny v dokumentu?', body: 'Dokument &bdquo;Rozpočet&ldquo; má neuložené změny. Zahozené změny najdete v historii souboru 30 dní.', opener: e.currentTarget,
      btns: [{ t: 'Zrušit', cancel: 1 }, { t: 'Zahodit', fn: function () { toast('info', 'Změny zahozeny', 'Zpět'); } },
        { t: 'Uložit', cls: 'pri', fn: function () { toast('ok', 'Uloženo'); } }] });
  });

  // ================= Oznameni =================
  var TI = { info: 'info', ok: 'check', warn: 'alert', dng: 'error' };
  function toast(kind, text, action) {
    var box = $('#toasts');
    var list = $$('.k-toast:not(.out)', box);
    if (list.length >= 3) out(list[0]);
    var t = document.createElement('div'); t.className = 'k-toast ' + kind; t.setAttribute('role', 'status');
    t.innerHTML = '<span class="lead-ic">' + I(TI[kind], 's') + '</span><span class="t">' + text + '</span>' +
      (action ? '<button class="k-btn k-f" data-a>' + action + '</button>' : '') +
      '<button class="k-btn ico ghost k-f" data-x aria-label="Zavřít oznámení" style="border-color:transparent">' + I('close', 's') + '</button>';
    box.appendChild(t);
    var left = action ? 8000 : 5000, start = Date.now(), timer = setTimeout(go, left), paused = false;
    function go() { out(t); }
    function pause() { if (paused) return; paused = true; clearTimeout(timer); left -= Date.now() - start; }
    function resume() { if (!paused || t.contains(document.activeElement)) return; paused = false; start = Date.now(); timer = setTimeout(go, Math.max(1500, left)); }
    t.addEventListener('mouseenter', pause); t.addEventListener('mouseleave', resume);
    t.addEventListener('focusin', pause); t.addEventListener('focusout', function () { setTimeout(resume, 0); });
    t.addEventListener('keydown', function (e) { if (e.key === 'Escape') { e.stopPropagation(); out(t); } });
    $('[data-x]', t).addEventListener('click', function () { out(t); });
    if (action) $('[data-a]', t).addEventListener('click', function () { log('Oznámení: ' + action); out(t); });
  }
  function out(t) {
    if (t.classList.contains('out')) return;
    t.classList.add('out'); setTimeout(function () { t.remove(); }, 200);
  }
  $('#t-ok').addEventListener('click', function () { toast('ok', 'Dokument uložen'); });
  $('#t-undo').addEventListener('click', function () { toast('info', '3 soubory přesunuty do koše', 'Zpět'); });
  $('#t-err').addEventListener('click', function () { toast('dng', 'Nelze se připojit k serveru', 'Zkusit znovu'); });

  // ================= Vyber v seznamu a tabulce =================
  // Spolecna logika: kurzor (cur) a vyber (aria-selected) jsou oddelene.
  function selectable(box, rows, names, onOpen) {
    var cur = 0, anchor = 0;
    function en(i) { return !rows()[i].classList.contains('k-dis'); }
    function paint() { rows().forEach(function (r, i) { r.classList.toggle('cur', i === cur); }); var r = rows()[cur]; if (r) r.scrollIntoView({ block: 'nearest' }); }
    function selOnly(i) { rows().forEach(function (r, j) { r.setAttribute('aria-selected', j === i); }); }
    function range(a, b) { var lo = Math.min(a, b), hi = Math.max(a, b); rows().forEach(function (r, j) { if (en(j)) r.setAttribute('aria-selected', j >= lo && j <= hi); }); }
    function count() { return rows().filter(function (r) { return r.getAttribute('aria-selected') === 'true'; }).length; }
    function step(from, d) {
      var n = rows().length, i = from;
      for (var k = 0; k < n; k++) { i = Math.max(0, Math.min(n - 1, i + d)); if (en(i)) return i; if (i === 0 || i === n - 1) break; }
      return from;
    }
    function move(to, e) {
      cur = to;
      if (e.shiftKey) range(anchor, cur); else if (!e.ctrlKey) { selOnly(cur); anchor = cur; }
      paint(); log('Vybráno: ' + count() + ', kurzor: ' + names()[cur]);
    }
    box.addEventListener('keydown', function (e) {
      var k = e.key, n = rows().length, page = 5;
      var to = k === 'ArrowDown' ? step(cur, 1) : k === 'ArrowUp' ? step(cur, -1) : k === 'Home' ? step(-1, 1) : k === 'End' ? step(n, -1)
        : k === 'PageDown' ? step(Math.min(n - 1, cur + page) - 1, 1) : k === 'PageUp' ? step(Math.max(0, cur - page) + 1, -1) : -1;
      if (to >= 0) { e.preventDefault(); move(to, e); return; }
      if (k === ' ') {
        e.preventDefault(); if (!en(cur)) return;
        var r = rows()[cur];
        if (e.ctrlKey) r.setAttribute('aria-selected', r.getAttribute('aria-selected') !== 'true'); else selOnly(cur);
        anchor = cur; log('Vybráno: ' + count()); return;
      }
      if (k === 'a' && e.ctrlKey) { e.preventDefault(); rows().forEach(function (r, j) { if (en(j)) r.setAttribute('aria-selected', 'true'); }); log('Vybráno vše: ' + count()); return; }
      if (k === 'Enter') { e.preventDefault(); if (onOpen) onOpen(cur); return; }
      if (k.length === 1 && !e.ctrlKey && !e.altKey) { var j = typeAhead(k, names(), cur); if (j >= 0 && en(j)) move(j, { shiftKey: false, ctrlKey: false }); }
    });
    box.addEventListener('mousedown', function (e) {
      var r = e.target.closest('[aria-selected]'); if (!r || !box.contains(r)) return;
      var i = rows().indexOf(r); if (i < 0 || !en(i)) return;
      cur = i;
      if (e.shiftKey) { e.preventDefault(); range(anchor, cur); }
      else if (e.ctrlKey || e.metaKey) { r.setAttribute('aria-selected', r.getAttribute('aria-selected') !== 'true'); anchor = cur; }
      else { selOnly(cur); anchor = cur; }
      paint(); log('Vybráno: ' + count());
    });
    box.addEventListener('dblclick', function () { if (onOpen) onOpen(cur); });
    selOnly(0); paint();
    return { reset: function () { cur = 0; anchor = 0; selOnly(0); paint(); } };
  }

  var FOLD = [['folder', 'Dokumenty', '1 204'], ['image', 'Obrázky', '8 311'], ['music', 'Hudba', '642'], ['video', 'Videa', '57'],
    ['download', 'Stažené', '93'], ['code', 'Projekty', '12 540'], ['server', 'Síťový disk', 'odpojeno', 'dis'], ['error', 'Zálohy', 'chyba čtení', 'err'], ['archive', 'Archiv', '3 020']];
  var lst = $('#lst');
  lst.innerHTML = FOLD.map(function (f) {
    return '<li class="k-li' + (f[3] === 'dis' ? ' k-dis' : f[3] === 'err' ? ' err' : '') + '" role="option" aria-selected="false"' + (f[3] === 'dis' ? ' aria-disabled="true"' : '') + '>' +
      I(f[0]) + f[1] + '<span class="sub">' + f[2] + '</span></li>';
  }).join('');
  selectable(lst, function () { return $$('.k-li', lst); }, function () { return FOLD.map(function (f) { return f[1]; }); },
    function (i) { log('Otevřít: ' + FOLD[i][1]); });

  var PROC = [['compd', 182, 3.1, 'Běží'], ['fsd', 96, 0.4, 'Běží'], ['netd', 41, 0, 'Spí'], ['Editor', 512, 12.5, 'Běží'], ['Prohlížeč', 1640, 8.2, 'Běží'],
    ['Přehrávač', 230, 0, 'Neodpovídá'], ['termd', 28, 0.1, 'Spí'], ['agentd', 350, 4, 'Běží']];
  var grd = $('#grd'), sortC = 0, sortD = 1;
  function drawGrid() {
    PROC.sort(function (a, b) { var x = a[sortC], y = b[sortC]; return (typeof x === 'number' ? x - y : x.localeCompare(y, 'cs')) * sortD; });
    $('tbody', grd).innerHTML = PROC.map(function (p) {
      var bad = p[3] === 'Neodpovídá';
      return '<tr role="row" aria-selected="false"' + (bad ? ' class="bad"' : '') + '><td>' + p[0] + '</td><td class="r">' + p[1].toLocaleString('cs') + ' MB</td><td class="r">' +
        p[2].toLocaleString('cs', { minimumFractionDigits: 1 }) + ' %</td><td>' + (bad ? I('alert', 's') + ' ' : '') + p[3] + '</td></tr>';
    }).join('');
    $$('th', grd).forEach(function (th, i) {
      th.setAttribute('aria-sort', i === sortC ? (sortD > 0 ? 'ascending' : 'descending') : 'none');
      $('use', th).setAttribute('href', '#ic-' + (i === sortC && sortD < 0 ? 'down' : 'up'));
    });
  }
  drawGrid();
  var gsel = selectable(grd, function () { return $$('tbody tr', grd); }, function () { return PROC.map(function (p) { return p[0]; }); },
    function (i) { log('Podrobnosti procesu: ' + PROC[i][0]); });
  $$('th', grd).forEach(function (th, i) {
    th.addEventListener('click', function () {
      if (sortC === i) sortD = -sortD; else { sortC = i; sortD = 1; }
      drawGrid(); gsel.reset(); log('Třídění: ' + th.textContent.trim() + (sortD > 0 ? ' vzestupně' : ' sestupně'));
    });
  });

  // ================= Strom =================
  var TREE = { t: 'Domů', c: [
    { t: 'Dokumenty', o: 1, c: [{ t: 'Práce', c: [{ t: 'Faktury', c: [] }, { t: 'Smlouvy', c: [] }] }, { t: 'Osobní', c: [] }, { t: 'Zprávy.odt', f: 1 }] },
    { t: 'Obrázky', c: [{ t: '2025', c: [] }, { t: '2026', c: [] }] },
    { t: 'Hudba', c: [] }, { t: 'Síťový disk', err: 1 }
  ] };
  var tree = $('#tree'), tcur = null;
  function tnode(n, lvl) {
    var kids = n.c && n.c.length;
    var li = '<li role="treeitem" aria-level="' + lvl + '" aria-selected="false"' + (kids ? ' aria-expanded="' + !!(n.o || lvl === 1) + '"' : '') + '>' +
      '<div class="k-tn" style="padding-left:' + ((lvl - 1) * 20 + 4) + 'px' + (n.err ? ';color:var(--c-danger)' : '') + '"><span class="tw' + (kids ? '' : ' leaf') + '">' + I('right', 's') + '</span>' +
      I(n.err ? 'error' : n.f ? 'file-text' : 'folder', n.err || n.f ? '' : 'f') + '<span>' + n.t + (n.err ? ' (nedostupné)' : '') + '</span></div>';
    if (kids) li += '<ul role="group">' + n.c.map(function (k) { return tnode(k, lvl + 1); }).join('') + '</ul>';
    return li + '</li>';
  }
  tree.innerHTML = tnode(TREE, 1);
  function visible() { return $$('[role="treeitem"]', tree).filter(function (li) { return li.offsetParent !== null; }); }
  function tfocus(li) {
    $$('[role="treeitem"]', tree).forEach(function (x) { x.tabIndex = -1; x.classList.remove('cur'); x.setAttribute('aria-selected', 'false'); });
    tcur = li; li.tabIndex = 0; li.classList.add('cur'); li.setAttribute('aria-selected', 'true'); li.focus();
    $('.k-tn', li).scrollIntoView({ block: 'nearest' });
  }
  function tname(li) { return $('.k-tn span:last-child', li).textContent; }
  function tset(li, v) { if (li.hasAttribute('aria-expanded')) { li.setAttribute('aria-expanded', v); log((v ? 'Rozbaleno: ' : 'Sbaleno: ') + tname(li)); } }
  tree.addEventListener('keydown', function (e) {
    var v = visible(), i = v.indexOf(tcur), k = e.key, ex = tcur.getAttribute('aria-expanded');
    if (k === 'ArrowDown' && i < v.length - 1) tfocus(v[i + 1]);
    else if (k === 'ArrowUp' && i > 0) tfocus(v[i - 1]);
    else if (k === 'Home') tfocus(v[0]);
    else if (k === 'End') tfocus(v[v.length - 1]);
    else if (k === 'ArrowRight') { if (ex === 'false') tset(tcur, true); else if (ex === 'true') tfocus($('[role="treeitem"]', tcur)); }
    else if (k === 'ArrowLeft') { if (ex === 'true') tset(tcur, false); else { var p = tcur.parentNode.closest('[role="treeitem"]'); if (p) tfocus(p); } }
    else if (k === '*') { $$(':scope > [role="treeitem"]', tcur.parentNode).forEach(function (s) { tset(s, true); }); }
    else if (k === 'Enter') log('Otevřít: ' + tname(tcur));
    else if (k.length === 1 && !e.ctrlKey) { var j = typeAhead(k, v.map(tname), i); if (j >= 0) tfocus(v[j]); return; }
    else return;
    e.preventDefault();
  });
  tree.addEventListener('click', function (e) {
    var li = e.target.closest('[role="treeitem"]'); if (!li) return;
    if (e.target.closest('.tw')) tset(li, li.getAttribute('aria-expanded') !== 'true');
    tfocus(li);
  });
  $$('[role="treeitem"]', tree).forEach(function (x) { x.tabIndex = -1; });
  var first = $('[role="treeitem"]', tree); first.tabIndex = 0; first.classList.add('cur'); first.setAttribute('aria-selected', 'true'); tcur = first;

  // ================= Prubeh =================
  var pr = $('#pr'), prT = 0;
  $('#pr-go').addEventListener('click', function () {
    clearInterval(prT); var v = 0; pr.classList.remove('ok');
    prT = setInterval(function () {
      v = Math.min(100, v + 4 + Math.round(Math.random() * 6));
      $('i', pr).style.width = v + '%'; pr.setAttribute('aria-valuenow', v); $('#pr-v').textContent = v + ' %';
      if (v >= 100) { clearInterval(prT); pr.classList.add('ok'); toast('ok', 'Kopírování dokončeno'); }
    }, 180);
  });

  // ================= Stitky =================
  var TAGS = [['Dokumenty', 0], ['Obrázky', 1], ['Tento týden', 0], ['Velké soubory', 0]];
  var tagBox = $('#tags');
  tagBox.innerHTML = TAGS.map(function (t, i) {
    return '<span class="k-tag k-f" role="button" tabindex="' + (i ? -1 : 0) + '" aria-pressed="' + !!t[1] + '">' + t[0] +
      '<button class="x" tabindex="-1" aria-label="Odebrat filtr ' + t[0] + '">' + I('close') + '</button></span>';
  }).join('');
  function tagList() { return $$('.k-tag', tagBox); }
  function tagFocus(t) { tagList().forEach(function (x) { x.tabIndex = x === t ? 0 : -1; }); t.focus(); }
  function tagRemove(t) {
    var l = tagList(), i = l.indexOf(t), next = l[i + 1] || l[i - 1];
    log('Filtr odebrán: ' + t.firstChild.textContent); t.remove();
    if (next) tagFocus(next);
  }
  tagBox.addEventListener('click', function (e) {
    var t = e.target.closest('.k-tag'); if (!t) return;
    if (e.target.closest('.x')) { tagRemove(t); return; }
    var v = t.getAttribute('aria-pressed') !== 'true'; t.setAttribute('aria-pressed', v); tagFocus(t);
    log('Filtr ' + t.firstChild.textContent + ': ' + (v ? 'zapnut' : 'vypnut'));
  });
  tagBox.addEventListener('keydown', function (e) {
    var t = e.target.closest('.k-tag'); if (!t) return;
    var l = tagList(), i = l.indexOf(t);
    if (e.key === 'ArrowRight') tagFocus(l[(i + 1) % l.length]);
    else if (e.key === 'ArrowLeft') tagFocus(l[(i - 1 + l.length) % l.length]);
    else if (e.key === ' ' || e.key === 'Enter') t.click();
    else if (e.key === 'Delete' || e.key === 'Backspace') tagRemove(t);
    else return;
    e.preventDefault();
  });

  // ================= Rozbalovaci panel =================
  var accB = $$('#accs .k-acc-b');
  accB.forEach(function (b, i) {
    b.addEventListener('click', function () {
      var v = b.getAttribute('aria-expanded') !== 'true'; b.setAttribute('aria-expanded', v);
      log((v ? 'Rozbaleno: ' : 'Sbaleno: ') + b.textContent.trim());
    });
    b.addEventListener('keydown', function (e) {
      var to = { ArrowDown: accB[(i + 1) % accB.length], ArrowUp: accB[(i - 1 + accB.length) % accB.length], Home: accB[0], End: accB[accB.length - 1] }[e.key];
      if (to) { e.preventDefault(); to.focus(); }
    });
  });

  strips();
})();
