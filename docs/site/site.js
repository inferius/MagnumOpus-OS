// Vyhledavani a prepinani motivu v HTML dokumentaci.
// Index (window.SEARCH_INDEX) generuje `cargo xtask docs` do search-<jazyk>.js.

(function () {
  "use strict";

  // Motiv: svetly / tmavy / podle systemu, pamatuje si ho prohlizec.
  const root = document.documentElement;
  function applyTheme(t) {
    if (t === "light" || t === "dark") root.setAttribute("data-theme", t);
    else root.removeAttribute("data-theme");
  }
  try { applyTheme(localStorage.getItem("os-docs-theme")); } catch (e) {}
  const themeBtn = document.getElementById("theme-toggle");
  if (themeBtn) {
    themeBtn.addEventListener("click", function () {
      const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
      const current = root.getAttribute("data-theme") || (dark ? "dark" : "light");
      const next = current === "dark" ? "light" : "dark";
      applyTheme(next);
      try { localStorage.setItem("os-docs-theme", next); } catch (e) {}
    });
  }

  // Vyhledavani: nadpisy a text sekci, bez ohledu na diakritiku a velikost pismen.
  const input = document.getElementById("search");
  const box = document.getElementById("search-results");
  const index = window.SEARCH_INDEX || [];
  if (!input || !box) return;

  function fold(s) {
    return s.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "");
  }
  const prepared = index.map(function (e) {
    return { e: e, title: fold(e.title), text: fold(e.text) };
  });

  let active = -1;
  function render(q) {
    box.innerHTML = "";
    active = -1;
    if (q.length < 2) { box.style.display = "none"; return; }
    const words = fold(q).split(/\s+/).filter(Boolean);
    const hits = [];
    for (const p of prepared) {
      let score = 0;
      let ok = true;
      for (const w of words) {
        if (p.title.includes(w)) score += 10;
        else if (p.text.includes(w)) score += 1;
        else { ok = false; break; }
      }
      if (ok) hits.push({ p: p, score: score });
    }
    hits.sort(function (a, b) { return b.score - a.score; });
    for (const h of hits.slice(0, 30)) {
      const a = document.createElement("a");
      a.href = h.p.e.url;
      a.textContent = h.p.e.title;
      const small = document.createElement("small");
      small.textContent = h.p.e.page;
      a.appendChild(small);
      box.appendChild(a);
    }
    box.style.display = hits.length ? "block" : "none";
  }

  input.addEventListener("input", function () { render(input.value.trim()); });
  input.addEventListener("keydown", function (ev) {
    const items = box.querySelectorAll("a");
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      ev.preventDefault();
      if (!items.length) return;
      if (active >= 0) items[active].classList.remove("active");
      active = (active + (ev.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
      items[active].classList.add("active");
    } else if (ev.key === "Enter" && items.length) {
      window.location.href = items[Math.max(active, 0)].href;
    } else if (ev.key === "Escape") {
      box.style.display = "none";
    }
  });
  document.addEventListener("click", function (ev) {
    if (!box.contains(ev.target) && ev.target !== input) box.style.display = "none";
  });
  document.addEventListener("keydown", function (ev) {
    if (ev.key === "/" && document.activeElement !== input) { ev.preventDefault(); input.focus(); }
  });
})();
