//! Generator HTML dokumentace: `cargo xtask docs`.
//!
//! Zdrojem pravdy zustavaji markdown soubory v `docs/cs` a `docs/en` (ctou
//! se i v gitu a hlida je test v crate abi). Generator z nich udela web
//! o dvou knihach (`docs_books.rs`: Prirucka a Pro vyvojare; kazda ma
//! vlastni menu a vyhledavani, v zahlavi je prepinac knih). Kazda kniha:
//! levy panel se strankami, obsah stranky, prepinac jazyka, vyhledavani,
//! svetly a tmavy motiv. Seznam stranek a jejich parovani mezi jazyky bere
//! z tabulky v `docs/README.md`. Styl a skript jsou v `docs/site/`.
//!
//! Generator zaroven kontroluje odkazy: odkaz na neexistujici stranku nebo
//! kotvu je chyba.
//!
//! Slozka `design/` (prirucka grafickeho jazyka, prototypy, `assets/`,
//! `tokens.toml`) se kopiruje do `target/docs/design/`, tedy vedle `cs/`
//! a `en/`. Markdown v `docs/cs` a `docs/en` na ni odkazuje relativne
//! k repozitari (`../../design/x.html`), aby odkaz fungoval i v gitu; na
//! webu lezi stranky o uroven mel, proto generator prepise predponu
//! `../../design/` na `../design/` a zkontroluje, ze cilovy soubor existuje.
//! Varianta s kopii primo do `cs/design/` a `en/design/` by zdvojila data
//! a varianta s ponechanim `../../design/` by vedla mimo `target/docs`.
//! Prirucka (`docs/prirucka/cs`) je o uroven hloubeji, v markdownu pouziva
//! `../../../design/` a na webu `../../design/`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

use crate::Result;
use crate::docs_books::{BOOKS, BookDef, PageDef, read_index};

const LANGS: [(&str, &str); 2] = [("cs", "Česky"), ("en", "English")];

/// Vyrenderovana stranka.
struct Rendered {
    file: String,
    title: String,
    html: String,
    toc: Vec<(u8, String, String)>,
    anchors: HashSet<String>,
    links: Vec<String>,
    sections: Vec<(String, String, String)>,
    /// Chyby nalezene pri vykresleni (napr. priklad shellu nejde prevest).
    errors: Vec<String>,
}

/// Vyrenderovana kniha v jednom jazyce.
struct Built {
    book: usize,
    li: usize,
    pages: std::rc::Rc<Vec<PageDef>>,
    rendered: Vec<Rendered>,
}

/// Vytvori web do `out` a vrati cestu k uvodni strance.
pub fn build(root: &Path, out: &Path) -> Result<PathBuf> {
    let docs = root.join("docs");
    fs::create_dir_all(out.join("assets"))?;
    // Styl webu + barvy shellu z motivu terminalu (jeden zdroj barev).
    let mut css = fs::read_to_string(docs.join("site/style.css"))?;
    css.push_str(&crate::highlight::shell_css());
    fs::write(out.join("assets/style.css"), css)?;
    fs::copy(docs.join("site/site.js"), out.join("assets/site.js"))?;
    copy_design(&root.join("design"), &out.join("design"))?;
    // Stare vystupy prirucky nesmi zustat (stranka, ktera uz v README neni).
    for b in BOOKS.iter().filter(|b| !b.out_sub.is_empty()) {
        let dir = out.join(b.out_sub);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
    }

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut built: Vec<Built> = Vec::new();
    for (bi, book) in BOOKS.iter().enumerate() {
        let src = root.join(book.src);
        let pages = std::rc::Rc::new(read_index(&src.join("README.md"))?);
        let mut h2: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for (li, (lang, _)) in LANGS.iter().enumerate() {
            let mut rendered = Vec::new();
            for page in pages.iter() {
                let md = fs::read_to_string(src.join(lang).join(&page.files[li]))
                    .map_err(|e| format!("{}/{lang}/{}: {e}", book.src, page.files[li]))?;
                crate::docs_check::page(lang, &page.files[li], &md, &mut errors, &mut warnings);
                rendered.push(render(&page.files[li], &md, book));
            }
            for r in &rendered {
                for e in &r.errors {
                    errors.push(format!("{}/{lang}/{}: {e}", book.id(), r.file));
                }
            }
            h2[li] = rendered.iter().map(|r| r.toc.iter().filter(|t| t.0 == 2).count()).collect();
            check_design_links(&book.web_dir(lang), &rendered, book, &root.join("design"), &mut errors);
            built.push(Built { book: bi, li, pages: pages.clone(), rendered });
        }
        for (pi, page) in pages.iter().enumerate() {
            crate::docs_check::pair(&page.files[0], h2[0][pi], &page.files[1], h2[1][pi], &mut warnings);
        }
    }

    // Odkazy se kontroluji az po vyrenderovani vsech knih (odkazy mezi knihami).
    let known = known_pages(&built);
    for b in &built {
        check_links(&BOOKS[b.book].web_dir(LANGS[b.li].0), &b.rendered, &known, &mut errors);
    }

    for b in &built {
        let book = &BOOKS[b.book];
        let lang = LANGS[b.li].0;
        let dir = out.join(book.web_dir(lang));
        fs::create_dir_all(&dir)?;
        let titles: Vec<String> = b.rendered.iter().map(|r| r.title.clone()).collect();
        for (pi, r) in b.rendered.iter().enumerate() {
            let other = &b.pages[pi].files[1 - b.li];
            let html = page_html(b.book, b.li, &b.pages, &titles, pi, r, other);
            fs::write(dir.join(html_name(&r.file)), html)?;
        }
        fs::write(dir.join("index.html"), index_html(b.book, b.li, &b.pages, &titles, &b.rendered))?;
        fs::write(out.join("assets").join(book.search_file(lang)), search_index(&b.rendered))?;
    }
    // Vychozi stranka webu je prirucka (cesky).
    let home = format!("{}/index.html", BOOKS[0].web_dir("cs"));
    fs::write(
        out.join("index.html"),
        format!(
            "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0; url={home}\"><title>Dokumentace</title><a href=\"{home}\">Dokumentace</a>\n"
        ),
    )?;
    if !warnings.is_empty() {
        println!("xtask: dokumentace: {} varovani (pravidla v docs/cs/dokumentace.md):", warnings.len());
        // Prvnich par cele, zbytek jen pocet podle souboru (stara
        // dokumentace jich ma stovky; audit je postupne odstrani).
        for w in warnings.iter().take(10) {
            println!("  {w}");
        }
        let mut per_file: std::collections::BTreeMap<&str, usize> = Default::default();
        for w in warnings.iter().skip(10) {
            *per_file.entry(w.split(':').next().unwrap_or("")).or_default() += 1;
        }
        for (f, n) in per_file {
            println!("  {f}: {n} dalsich");
        }
    }
    if !errors.is_empty() {
        return Err(format!("chyby v dokumentaci:\n  {}", errors.join("\n  ")).into());
    }
    Ok(out.join("index.html"))
}

/// Vsechny stranky vsech knih: webova cesta (`prirucka/cs/x.html`) -> kotvy.
fn known_pages(built: &[Built]) -> HashMap<String, &HashSet<String>> {
    let mut map = HashMap::new();
    for b in built {
        let dir = BOOKS[b.book].web_dir(LANGS[b.li].0);
        for r in &b.rendered {
            map.insert(format!("{dir}/{}", html_name(&r.file)), &r.anchors);
        }
    }
    map
}

fn html_name(md: &str) -> String {
    md.trim_end_matches(".md").to_string() + ".html"
}

/// Kotva nadpisu ve stylu GitHubu (aby fungovaly stejne odkazy jako v gitu).
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render(file: &str, md: &str, book: &BookDef) -> Rendered {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out_events: Vec<Event> = Vec::new();
    let mut heading: Option<(HeadingLevel, Vec<Event>)> = None;
    let mut title = String::new();
    let mut toc = Vec::new();
    let mut anchors = HashSet::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut links = Vec::new();
    let mut sections: Vec<(String, String, String)> = Vec::new();
    let mut doc_errors: Vec<String> = Vec::new();

    // Blok kodu: jazyk a dosud nasbirany text (zvyrazni se najednou na konci).
    let mut code: Option<(String, String)> = None;
    for ev in Parser::new_ext(md, opts) {
        match (&mut code, &ev) {
            (None, Event::Start(Tag::CodeBlock(kind))) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => {
                        l.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
                continue;
            }
            (Some((_, buf)), Event::Text(t)) => {
                buf.push_str(t);
                continue;
            }
            (Some((lang, _)), Event::End(TagEnd::CodeBlock))
                if lang == "shell-b" || lang == "shell-c" =>
            {
                let (lang, buf) = code.take().unwrap();
                match shell_pair(&lang, &buf) {
                    Ok(html) => out_events.push(Event::Html(CowStr::from(html))),
                    Err(e) => doc_errors.push(e),
                }
                continue;
            }
            (Some(_), Event::End(TagEnd::CodeBlock)) => {
                let (lang, buf) = code.take().unwrap();
                let class = if lang.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"language-{lang}\"")
                };
                out_events.push(Event::Html(CowStr::from(format!(
                    "<pre><code{class}>{}</code></pre>\n",
                    crate::highlight::highlight(&lang, &buf)
                ))));
                continue;
            }
            _ => {}
        }
        let ev = match ev {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_link(&dest_url, book);
                if !dest.starts_with("http") {
                    links.push(dest.clone());
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url: CowStr::from(dest),
                    title,
                    id,
                })
            }
            // Obrazky (snimky v `design/snimky/`): stejny prepis cesty
            // a kontrola existence jako u odkazu.
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_link(&dest_url, book);
                if !dest.starts_with("http") {
                    links.push(dest.clone());
                }
                Event::Start(Tag::Image {
                    link_type,
                    dest_url: CowStr::from(dest),
                    title,
                    id,
                })
            }
            other => other,
        };
        match (&mut heading, ev) {
            (None, Event::Start(Tag::Heading { level, .. })) => heading = Some((level, Vec::new())),
            (Some(_), Event::End(TagEnd::Heading(_))) => {
                let (level, inner) = heading.take().unwrap();
                let text: String = inner
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                        _ => None,
                    })
                    .collect();
                let base = slug(&text);
                let n = used.entry(base.clone()).or_insert(0);
                let id = if *n == 0 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                };
                *n += 1;
                anchors.insert(id.clone());
                let mut inner_html = String::new();
                html::push_html(&mut inner_html, inner.into_iter());
                let lvl = level as u8;
                if lvl == 1 && title.is_empty() {
                    title = text.clone();
                }
                if lvl == 2 || lvl == 3 {
                    toc.push((lvl, id.clone(), text.clone()));
                }
                sections.push((text.clone(), id.clone(), String::new()));
                let anchor = if lvl > 1 {
                    format!("<a class=\"anchor\" href=\"#{id}\">#</a>")
                } else {
                    String::new()
                };
                out_events.push(Event::Html(CowStr::from(format!(
                    "<h{lvl} id=\"{id}\">{inner_html}{anchor}</h{lvl}>\n"
                ))));
            }
            (Some((_, inner)), ev) => inner.push(ev),
            (None, ev) => {
                if let (Event::Text(t) | Event::Code(t), Some(last)) = (&ev, sections.last_mut())
                    && last.2.len() < 600
                {
                    last.2.push_str(t);
                    last.2.push(' ');
                }
                out_events.push(ev);
            }
        }
    }
    let mut body = String::new();
    html::push_html(&mut body, out_events.into_iter());
    Rendered {
        file: file.to_string(),
        title,
        html: crate::docs_check::chips(&body),
        toc,
        anchors,
        links,
        sections,
        errors: doc_errors,
    }
}

/// Priklad shellu v obou zapisech: napsany (`shell-b` nebo `shell-c`)
/// a druhy vygenerovany formatovacem. Priklad, ktery nejde prevest tam
/// a zpet na stejny strom, je chyba dokumentace.
fn shell_pair(lang: &str, src: &str) -> std::result::Result<String, String> {
    use shlang::{Style, fmt, parse, sig::Builtins};
    let from = if lang == "shell-c" { Style::C } else { Style::B };
    let opts = shlang::Options { style: Some(from), sigs: &Builtins, vars: Vec::new(), script: false };
    let parsed = parse(src, &opts).map_err(|e| format!("priklad shellu nejde rozparsovat:
{}", e.report(src)))?;
    let fo = |style| fmt::FmtOptions { style, sigs: &Builtins, vars: Vec::new() };
    let b = fmt::format(&parsed.script, &fo(Style::B));
    let c = fmt::format(&parsed.script, &fo(Style::C));
    for (text, style) in [(&b, Style::B), (&c, Style::C)] {
        let again = parse(text, &shlang::Options { style: Some(style), sigs: &Builtins, vars: Vec::new(), script: false })
            .map_err(|e| format!("prevod prikladu shellu selhal:
{}", e.report(text)))?;
        if again.script != parsed.script {
            return Err(format!("priklad shellu dava po prevodu jiny strom:
{src}
---
{text}"));
        }
    }
    let block = |label: &str, cls: &str, text: &str| {
        format!(
            "<div class=\"style-{cls}\"><div class=\"style-label\">{label}</div><pre><code class=\"language-shell-{cls}\">{}</code></pre></div>",
            crate::highlight::highlight("shell-b", text.trim_end())
        )
    };
    // Napsany zapis prvni, vygenerovany druhy (oznaceny hvezdickou).
    let (first, second) = if from == Style::B {
        (block("B", "b", &b), block("C *", "c", &c))
    } else {
        (block("C", "c", &c), block("B *", "b", &b))
    };
    Ok(format!("<div class=\"two-styles\">{first}{second}</div>
"))
}

// Predpony odkazu do `design/` (v markdownu a na webu) jsou u knihy
// (`BookDef::design_src` a `design_web`): prirucka lezi o uroven hloubeji.

/// Podslozky `design/`, ktere na web nepatri (nastroje pro build tokenu).
const DESIGN_SKIP: [&str; 1] = ["tools"];

/// Zkopiruje `design/` do `out` (puvodni obsah `out` smaze, aby na webu
/// nezustaly soubory, ktere uz v repozitari nejsou). Bez slozky `design/`
/// se nic nekopiruje.
fn copy_design(src: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    if !src.is_dir() {
        return Ok(());
    }
    copy_dir(src, out, &DESIGN_SKIP)
}

/// Rekurzivni kopie slozky; `skip` jsou jmena podslozek na nejvyssi urovni,
/// ktere se vynechaji.
fn copy_dir(src: &Path, out: &Path, skip: &[&str]) -> Result<()> {
    fs::create_dir_all(out)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if skip.iter().any(|s| name == *s) {
                continue;
            }
            copy_dir(&path, &out.join(&name), &[])?;
        } else {
            fs::copy(&path, out.join(&name))?;
        }
    }
    Ok(())
}

/// `x.md` -> `x.html`, `x.md#a` -> `x.html#a`, `../../design/x` ->
/// `../design/x`; ostatni beze zmeny.
fn rewrite_link(dest: &str, book: &BookDef) -> String {
    if dest.starts_with("http") {
        return dest.to_string();
    }
    if let Some(rest) = dest.strip_prefix(book.design_src) {
        return format!("{}{rest}", book.design_web);
    }
    let (path, frag) = dest
        .split_once('#')
        .map_or((dest, None), |(p, f)| (p, Some(f)));
    let path = if let Some(stem) = path.strip_suffix(".md") {
        format!("{stem}.html")
    } else {
        path.to_string()
    };
    match frag {
        Some(f) => format!("{path}#{f}"),
        None => path,
    }
}

/// Zkontroluje odkazy stranek jedne knihy a jazyka. `dir` je webova slozka
/// stranek (`cs`, `prirucka/en`); odkazy se rozlozi proti ni, takze funguji
/// i odkazy mezi knihami (`../../cs/x.md` z prirucky). Odkazy mimo knihy
/// (napr. `design/`) se tu nekontroluji.
fn check_links(dir: &str, pages: &[Rendered], known: &HashMap<String, &HashSet<String>>, errors: &mut Vec<String>) {
    for page in pages {
        for link in &page.links {
            if link.starts_with("http") {
                continue;
            }
            let (path, frag) = link
                .split_once('#')
                .map_or((link.as_str(), None), |(p, f)| (p, Some(f)));
            if !path.is_empty() && !path.ends_with(".html") {
                continue;
            }
            let target = if path.is_empty() {
                Some(format!("{dir}/{}", html_name(&page.file)))
            } else {
                resolve_web(dir, path)
            };
            let Some(target) = target else {
                continue;
            };
            match (known.get(&target), frag) {
                (None, _) if target.ends_with("/index.html") => {}
                (None, _) if is_book_path(&target) => errors.push(format!(
                    "{dir}/{}: odkaz na neexistujici stranku {link}",
                    page.file
                )),
                (Some(a), Some(f)) if !a.contains(f) => errors.push(format!(
                    "{dir}/{}: odkaz na neexistujici kotvu {link}",
                    page.file
                )),
                _ => {}
            }
        }
    }
}

/// Spoji relativni odkaz se slozkou a odstrani `.` a `..`; `None`, kdyz
/// vede nad koren webu.
fn resolve_web(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Cesta (relativne ke koreni webu) lezi v nekterem jazyku nektere knihy.
fn is_book_path(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or("");
    LANGS.iter().any(|(l, _)| *l == first) || BOOKS.iter().any(|b| !b.out_sub.is_empty() && b.out_sub == first)
}

/// Odkazy do `design/` musi vest na existujici soubor (nebo slozku)
/// ve zdrojove `design/` mimo vynechane podslozky.
fn check_design_links(lang: &str, pages: &[Rendered], book: &BookDef, design: &Path, errors: &mut Vec<String>) {
    for page in pages {
        for link in &page.links {
            let Some(rest) = link.strip_prefix(book.design_web) else {
                continue;
            };
            let path = rest.split(['#', '?']).next().unwrap_or("");
            let skipped = DESIGN_SKIP.iter().any(|s| path.split('/').next() == Some(*s));
            if skipped || path.split('/').any(|c| c == "..") || !design.join(path).exists() {
                errors.push(format!(
                    "{lang}/{}: odkaz do design/ na neexistujici soubor {link}",
                    page.file
                ));
            }
        }
    }
}

fn sidebar(li: usize, pages: &[PageDef], titles: &[String], current: Option<usize>) -> String {
    let mut s = String::new();
    let heading = if li == 0 { "Obsah" } else { "Contents" };
    let _ = write!(s, "<nav class=\"sidebar\"><h4>{heading}</h4>");
    let home = if li == 0 { "Úvod" } else { "Overview" };
    let cls = if current.is_none() {
        " class=\"current\""
    } else {
        ""
    };
    let _ = write!(s, "<a href=\"index.html\"{cls}>{home}</a>");
    let mut last_group = "";
    for (i, p) in pages.iter().enumerate() {
        if p.group[li] != last_group && !p.group[li].is_empty() {
            last_group = &p.group[li];
            let _ = write!(s, "<h5>{}</h5>", escape(last_group));
        }
        let cls = if current == Some(i) {
            " class=\"current\""
        } else {
            ""
        };
        let name = if p.names[li].is_empty() { short_title(&titles[i]) } else { &p.names[li] };
        let _ = write!(
            s,
            "<a href=\"{}\"{cls} title=\"{}\">{}</a>",
            html_name(&p.files[li]),
            escape(&p.descriptions[li]),
            escape(name)
        );
    }
    s.push_str("</nav>");
    s
}

/// Kratky nazev do panelu: nadpis stranky do dvojtecky nebo pomlcky.
fn short_title(title: &str) -> &str {
    title.split([':', '-']).next().unwrap_or(title).trim()
}

fn shell(
    bi: usize,
    li: usize,
    title: &str,
    other_lang_href: &str,
    sidebar: &str,
    main: &str,
    toc: &str,
) -> String {
    let (lang, _) = LANGS[li];
    let (other, other_name) = LANGS[1 - li];
    let book = &BOOKS[bi];
    let root = book.root_prefix();
    let placeholder = if li == 0 { "Hledat (/)" } else { "Search (/)" };
    let theme = if li == 0 { "Motiv" } else { "Theme" };
    // Prepinac knih: odkaz na uvod kazde knihy ve stejnem jazyce.
    let mut books = String::from("<nav class=\"books\">");
    for (j, b) in BOOKS.iter().enumerate() {
        let cls = if j == bi { " class=\"current\"" } else { "" };
        let _ = write!(books, "<a href=\"{root}{}/index.html\"{cls}>{}</a>", b.web_dir(lang), b.names[li]);
    }
    books.push_str("</nav>");
    let search = book.search_file(lang);
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n<link rel=\"stylesheet\" href=\"{root}assets/style.css\">\n</head>\n<body>\n\
<header class=\"topbar\"><a class=\"brand\" href=\"index.html\">os <span>docs</span></a>{books}\
<div class=\"search\"><input id=\"search\" type=\"search\" placeholder=\"{placeholder}\" autocomplete=\"off\">\
<div id=\"search-results\" class=\"search-results\"></div></div>\
<div class=\"top-actions\"><a href=\"../{other}/{other_lang_href}\">{other_name}</a>\
<button id=\"theme-toggle\" type=\"button\">{theme}</button></div></header>\n\
<div class=\"layout\">{sidebar}<main><article class=\"content\">{main}</article></main>{toc}</div>\n\
<script src=\"{root}assets/{search}\"></script>\n<script src=\"{root}assets/site.js\"></script>\n</body>\n</html>\n",
        title = escape(title)
    )
}

fn page_html(
    bi: usize,
    li: usize,
    pages: &[PageDef],
    titles: &[String],
    pi: usize,
    r: &Rendered,
    other_file: &str,
) -> String {
    let mut toc = String::new();
    if !r.toc.is_empty() {
        let label = if li == 0 {
            "Na této stránce"
        } else {
            "On this page"
        };
        let _ = write!(toc, "<aside class=\"toc\"><h4>{label}</h4>");
        for (lvl, id, text) in &r.toc {
            let _ = write!(
                toc,
                "<a class=\"h{lvl}\" href=\"#{id}\">{}</a>",
                escape(text)
            );
        }
        toc.push_str("</aside>");
    }
    let src = format!("{}/{}", BOOKS[bi].src, LANGS[li].0);
    let label = if li == 0 { "Zdroj" } else { "Source" };
    let footer = format!("<div class=\"page-footer\">{label}: <code>{src}/{}</code></div>", r.file);
    shell(
        bi,
        li,
        &r.title,
        &html_name(other_file),
        &sidebar(li, pages, titles, Some(pi)),
        &(r.html.clone() + &footer),
        &toc,
    )
}

fn index_html(bi: usize, li: usize, pages: &[PageDef], titles: &[String], rendered: &[Rendered]) -> String {
    let (title, intro) = (BOOKS[bi].titles[li], BOOKS[bi].intros[li]);
    let mut main = format!("<h1>{title}</h1><p>{intro}</p>");
    let mut last_group: Option<&str> = None;
    for (p, r) in pages.iter().zip(rendered) {
        if last_group != Some(p.group[li].as_str()) {
            if last_group.is_some() {
                main.push_str("</tbody></table>");
            }
            last_group = Some(&p.group[li]);
            if !p.group[li].is_empty() {
                let _ = write!(main, "<h2>{}</h2>", escape(&p.group[li]));
            }
            main.push_str("<table><tbody>");
        }
        let _ = write!(
            main,
            "<tr><td><a href=\"{}\"><strong>{}</strong></a></td><td>{}</td></tr>",
            html_name(&p.files[li]),
            escape(&r.title),
            escape(&p.descriptions[li])
        );
    }
    if last_group.is_some() {
        main.push_str("</tbody></table>");
    }
    shell(
        bi,
        li,
        title,
        "index.html",
        &sidebar(li, pages, titles, None),
        &main,
        "",
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn search_index(pages: &[Rendered]) -> String {
    let mut s = String::from("window.SEARCH_INDEX = [\n");
    for p in pages {
        for (title, id, text) in &p.sections {
            let url = format!("{}#{id}", html_name(&p.file));
            let _ = writeln!(
                s,
                "{{\"title\":{},\"page\":{},\"url\":{},\"text\":{}}},",
                json_str(title),
                json_str(&p.title),
                json_str(&url),
                json_str(text.trim())
            );
        }
    }
    s.push_str("];\n");
    s
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
