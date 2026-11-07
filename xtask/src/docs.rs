//! Generator HTML dokumentace: `cargo xtask docs`.
//!
//! Zdrojem pravdy zustavaji markdown soubory v `docs/cs` a `docs/en` (ctou
//! se i v gitu a hlida je test v crate abi). Generator z nich udela web:
//! levy panel se strankami, obsah stranky, prepinac jazyka, vyhledavani,
//! svetly a tmavy motiv. Seznam stranek a jejich parovani mezi jazyky bere
//! z tabulky v `docs/README.md`. Styl a skript jsou v `docs/site/`.
//!
//! Generator zaroven kontroluje odkazy: odkaz na neexistujici stranku nebo
//! kotvu je chyba.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

use crate::Result;

const LANGS: [(&str, &str); 2] = [("cs", "Česky"), ("en", "English")];

/// Jedna stranka v obou jazycich.
struct PageDef {
    files: [String; 2],
    /// Nazev v menu (text odkazu v README).
    names: [String; 2],
    descriptions: [String; 2],
    /// Skupina v menu (nadpis `###` v README).
    group: [String; 2],
}

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

/// Vytvori web do `out` a vrati cestu k uvodni strance.
pub fn build(root: &Path, out: &Path) -> Result<PathBuf> {
    let docs = root.join("docs");
    let pages = read_index(&docs.join("README.md"))?;
    fs::create_dir_all(out.join("assets"))?;
    // Styl webu + barvy shellu z motivu terminalu (jeden zdroj barev).
    let mut css = fs::read_to_string(docs.join("site/style.css"))?;
    css.push_str(&crate::highlight::shell_css());
    fs::write(out.join("assets/style.css"), css)?;
    fs::copy(docs.join("site/site.js"), out.join("assets/site.js"))?;

    let mut errors = Vec::new();
    for (li, (lang, _)) in LANGS.iter().enumerate() {
        let dir = out.join(lang);
        fs::create_dir_all(&dir)?;
        let mut rendered = Vec::new();
        for page in &pages {
            let md = fs::read_to_string(docs.join(lang).join(&page.files[li]))
                .map_err(|e| format!("docs/{lang}/{}: {e}", page.files[li]))?;
            rendered.push(render(&page.files[li], &md));
        }
        for r in &rendered {
            for e in &r.errors {
                errors.push(format!("{lang}/{}: {e}", r.file));
            }
        }
        check_links(lang, &rendered, &mut errors);
        let titles: Vec<String> = rendered.iter().map(|r| r.title.clone()).collect();
        for (pi, r) in rendered.iter().enumerate() {
            let other = &pages[pi].files[1 - li];
            let html = page_html(li, &pages, &titles, pi, r, other);
            fs::write(dir.join(html_name(&r.file)), html)?;
        }
        fs::write(
            dir.join("index.html"),
            index_html(li, &pages, &titles, &rendered),
        )?;
        fs::write(
            out.join(format!("assets/search-{lang}.js")),
            search_index(&rendered),
        )?;
    }
    fs::write(
        out.join("index.html"),
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0; url=cs/index.html\"><title>Dokumentace</title><a href=\"cs/index.html\">Dokumentace</a>\n",
    )?;
    if !errors.is_empty() {
        return Err(format!("chyby v dokumentaci:\n  {}", errors.join("\n  ")).into());
    }
    Ok(out.join("index.html"))
}

/// Precte stranky z docs/README.md: skupiny `### cs / en` a radky
/// `| [Nazev](cs/x.md) | [Name](en/y.md) | popis cs / popis en |`.
fn read_index(path: &Path) -> Result<Vec<PageDef>> {
    let text = fs::read_to_string(path)?;
    let mut pages = Vec::new();
    let mut group = [String::new(), String::new()];
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            let (cs, en) = h.split_once(" / ").unwrap_or((h, h));
            group = [cs.trim().to_string(), en.trim().to_string()];
            continue;
        }
        if !line.starts_with("| [") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let target = |cell: &str| -> Option<String> {
            let start = cell.find("](")? + 2;
            let end = cell[start..].find(')')? + start;
            Some(cell[start..end].rsplit('/').next()?.to_string())
        };
        let (Some(cs), Some(en)) = (target(cells[0]), target(cells[1])) else {
            continue;
        };
        let name = |cell: &str| -> String {
            cell.find("](").map_or(String::new(), |e| cell[1..e].trim_start_matches('[').to_string())
        };
        let (dcs, den) = cells[2].split_once(" / ").unwrap_or((cells[2], cells[2]));
        pages.push(PageDef {
            files: [cs, en],
            names: [name(cells[0]), name(cells[1])],
            descriptions: [dcs.to_string(), den.to_string()],
            group: group.clone(),
        });
    }
    if pages.is_empty() {
        return Err("docs/README.md neobsahuje tabulku stranek".into());
    }
    Ok(pages)
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

fn render(file: &str, md: &str) -> Rendered {
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
                let dest = rewrite_link(&dest_url);
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
        html: body,
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

/// `x.md` -> `x.html`, `x.md#a` -> `x.html#a`; ostatni beze zmeny.
fn rewrite_link(dest: &str) -> String {
    if dest.starts_with("http") {
        return dest.to_string();
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

fn check_links(lang: &str, pages: &[Rendered], errors: &mut Vec<String>) {
    let by_name: HashMap<String, &Rendered> =
        pages.iter().map(|p| (html_name(&p.file), p)).collect();
    for page in pages {
        for link in &page.links {
            if link.starts_with("http") {
                continue;
            }
            let (path, frag) = link
                .split_once('#')
                .map_or((link.as_str(), None), |(p, f)| (p, Some(f)));
            let target = if path.is_empty() {
                Some(page)
            } else {
                by_name.get(path).copied()
            };
            match (target, frag) {
                (None, _) if path.contains('/') || !path.ends_with(".html") => {}
                (None, _) => errors.push(format!(
                    "{lang}/{}: odkaz na neexistujici stranku {link}",
                    page.file
                )),
                (Some(t), Some(f)) if !t.anchors.contains(f) => errors.push(format!(
                    "{lang}/{}: odkaz na neexistujici kotvu {link}",
                    page.file
                )),
                _ => {}
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
    li: usize,
    title: &str,
    other_lang_href: &str,
    sidebar: &str,
    main: &str,
    toc: &str,
) -> String {
    let (lang, _) = LANGS[li];
    let (other, other_name) = LANGS[1 - li];
    let placeholder = if li == 0 { "Hledat (/)" } else { "Search (/)" };
    let theme = if li == 0 { "Motiv" } else { "Theme" };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n<link rel=\"stylesheet\" href=\"../assets/style.css\">\n</head>\n<body>\n\
<header class=\"topbar\"><a class=\"brand\" href=\"index.html\">os <span>docs</span></a>\
<div class=\"search\"><input id=\"search\" type=\"search\" placeholder=\"{placeholder}\" autocomplete=\"off\">\
<div id=\"search-results\" class=\"search-results\"></div></div>\
<div class=\"top-actions\"><a href=\"../{other}/{other_lang_href}\">{other_name}</a>\
<button id=\"theme-toggle\" type=\"button\">{theme}</button></div></header>\n\
<div class=\"layout\">{sidebar}<main><article class=\"content\">{main}</article></main>{toc}</div>\n\
<script src=\"../assets/search-{lang}.js\"></script>\n<script src=\"../assets/site.js\"></script>\n</body>\n</html>\n",
        title = escape(title)
    )
}

fn page_html(
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
    let footer = if li == 0 {
        format!(
            "<div class=\"page-footer\">Zdroj: <code>docs/cs/{}</code></div>",
            r.file
        )
    } else {
        format!(
            "<div class=\"page-footer\">Source: <code>docs/en/{}</code></div>",
            r.file
        )
    };
    shell(
        li,
        &r.title,
        &html_name(other_file),
        &sidebar(li, pages, titles, Some(pi)),
        &(r.html.clone() + &footer),
        &toc,
    )
}

fn index_html(li: usize, pages: &[PageDef], titles: &[String], rendered: &[Rendered]) -> String {
    let (title, intro) = if li == 0 {
        (
            "Dokumentace operačního systému",
            "Nový operační systém psaný od nuly v Rustu: mikrokernel, capabilities, ovladače a služby v userspace, později Windows aplikace v izolované vrstvě.",
        )
    } else {
        (
            "Operating system documentation",
            "A new operating system written from scratch in Rust: a microkernel, capabilities, drivers and services in userspace, later Windows applications in an isolated layer.",
        )
    };
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
