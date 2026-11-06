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

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

use crate::Result;

const LANGS: [(&str, &str); 2] = [("cs", "Česky"), ("en", "English")];

/// Jedna stranka v obou jazycich.
struct PageDef {
    files: [String; 2],
    descriptions: [String; 2],
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
}

/// Vytvori web do `out` a vrati cestu k uvodni strance.
pub fn build(root: &Path, out: &Path) -> Result<PathBuf> {
    let docs = root.join("docs");
    let pages = read_index(&docs.join("README.md"))?;
    fs::create_dir_all(out.join("assets"))?;
    fs::copy(docs.join("site/style.css"), out.join("assets/style.css"))?;
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

/// Precte tabulku stranek z docs/README.md: `| [x](cs/x.md) | [y](en/y.md) | popis cs / popis en |`.
fn read_index(path: &Path) -> Result<Vec<PageDef>> {
    let text = fs::read_to_string(path)?;
    let mut pages = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("| [")) {
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
        let (dcs, den) = cells[2].split_once(" / ").unwrap_or((cells[2], cells[2]));
        pages.push(PageDef {
            files: [cs, en],
            descriptions: [dcs.to_string(), den.to_string()],
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

    for ev in Parser::new_ext(md, opts) {
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
    }
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
    for (i, p) in pages.iter().enumerate() {
        let cls = if current == Some(i) {
            " class=\"current\""
        } else {
            ""
        };
        let name = short_title(&titles[i]);
        let _ = write!(
            s,
            "<a href=\"{}\"{cls}>{}<small>{}</small></a>",
            html_name(&p.files[li]),
            escape(name),
            escape(&p.descriptions[li])
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
    let mut main = format!("<h1>{title}</h1><p>{intro}</p><table><tbody>");
    for (p, r) in pages.iter().zip(rendered) {
        let _ = write!(
            main,
            "<tr><td><a href=\"{}\"><strong>{}</strong></a></td><td>{}</td></tr>",
            html_name(&p.files[li]),
            escape(&r.title),
            escape(&p.descriptions[li])
        );
    }
    main.push_str("</tbody></table>");
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
