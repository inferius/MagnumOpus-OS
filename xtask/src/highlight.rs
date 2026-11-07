//! Zvyrazneni syntaxe v dokumentaci (pri generovani HTML, bez skriptu v
//! prohlizeci a bez zavislosti).
//!
//! Jednoduchy lexer: komentare, retezce a znaky, cisla, klicova slova,
//! typy (jmena s velkym pocatecnim pismenem), makra (`jmeno!`), atributy
//! (`#[...]`), lifetimy (`'a`). Podporovane jazyky: `rust`, `c`, `cpp`,
//! `toml`, `bash`/`sh`; ostatni (`text`, bez jazyka) zustavaji bez barev.
//! Vystup jsou `<span class="hl-...">`, barvy definuje `style.css`.
//!
//! Jazyk shellu (`shell-b`, `shell-c`) se nezvyraznuje tady: tokeny
//! a tridy dava `shlang::highlight` - stejne jako terminal ([`mos`]).
//! Tridy jsou `sh-<trida>`, barvy generuje motiv terminalu
//! (`term::theme`, viz [`shell_css`]).

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
const C_KW: &[&str] = &[
    "auto",
    "bool",
    "break",
    "case",
    "char",
    "class",
    "const",
    "constexpr",
    "continue",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "false",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "new",
    "nullptr",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "true",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];
const BASH_KW: &[&str] = &[
    "cd", "do", "done", "echo", "elif", "else", "esac", "export", "fi", "for", "function", "if",
    "in", "then", "while",
];
/// Typy psane malymi pismeny (Rust primitivni typy, C typy maji klic. slova).
const RUST_PRIM: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize",
];

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    C,
    Toml,
    Bash,
    /// Jazyk shellu MagnumOpus (oba zapisy).
    Mos,
}

fn lang_of(name: &str) -> Option<Lang> {
    match name {
        "rust" | "rs" => Some(Lang::Rust),
        "c" | "cpp" | "c++" | "h" => Some(Lang::C),
        "toml" => Some(Lang::Toml),
        "bash" | "sh" | "shell" | "powershell" => Some(Lang::Bash),
        // `mos` = shell bez prevodu do druheho zapisu (smiseny priklad, skript).
        "shell-b" | "shell-c" | "mos" => Some(Lang::Mos),
        _ => None,
    }
}

fn esc(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn span(out: &mut String, class: &str, text: &str) {
    out.push_str("<span class=\"hl-");
    out.push_str(class);
    out.push_str("\">");
    esc(out, text);
    out.push_str("</span>");
}

/// Jazyk shellu: useky z `shlang::highlight` (lexikalni uroven, jako radek
/// bloku v terminalu) -> `<span class="sh-...">`.
pub fn mos(code: &str) -> String {
    let h = shlang::highlight::highlight(code, None);
    let mut out = String::with_capacity(code.len() * 2);
    let mut at = 0;
    for sp in &h.spans {
        esc(&mut out, &code[at..sp.start]);
        out.push_str("<span class=\"sh-");
        out.push_str(sp.class.name());
        out.push_str("\">");
        esc(&mut out, &code[sp.start..sp.end]);
        out.push_str("</span>");
        at = sp.end;
    }
    esc(&mut out, &code[at..]);
    out
}

/// CSS barev shellu z motivu terminalu (svetly / tmavy jako zbytek webu).
pub fn shell_css() -> String {
    let (light, dark) = (term::theme::Theme::light(), term::theme::Theme::dark());
    let mut out = String::from("\n/* Shell: generuje xtask z term::theme (stejne barvy jako terminal). */\n");
    out.push_str(&light.css_vars(":root"));
    out.push_str("@media (prefers-color-scheme: dark) {\n");
    out.push_str(&dark.css_vars(":root:not([data-theme=\"light\"])"));
    out.push_str("}\n");
    out.push_str(&dark.css_vars(":root[data-theme=\"dark\"]"));
    out.push_str(&dark.css_rules());
    out
}

/// Zvyrazni `code` v jazyce `lang`; vraci HTML (bez obalu `<pre>`).
pub fn highlight(lang: &str, code: &str) -> String {
    let mut out = String::with_capacity(code.len() * 2);
    let Some(l) = lang_of(lang) else {
        esc(&mut out, code);
        return out;
    };
    if l == Lang::Mos {
        return mos(code);
    }
    let chars: Vec<char> = code.chars().collect();
    let text = |a: usize, b: usize| -> String { chars[a..b].iter().collect() };
    let n = chars.len();
    let mut i = 0;
    // Zacatek radku (TOML klice a sekce).
    let mut line_start = true;
    while i < n {
        let c = chars[i];
        let next = chars.get(i + 1).copied().unwrap_or('\0');
        // Komentare.
        let line_comment = match l {
            Lang::Rust | Lang::C => c == '/' && next == '/',
            Lang::Toml | Lang::Bash => c == '#',
            // Shell resi `mos` (vyse).
            Lang::Mos => false,
        };
        if line_comment {
            let end = chars[i..]
                .iter()
                .position(|&x| x == '\n')
                .map_or(n, |p| i + p);
            span(&mut out, "com", &text(i, end));
            i = end;
            continue;
        }
        if matches!(l, Lang::Rust | Lang::C) && c == '/' && next == '*' {
            let end = (i + 2..n.saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
                .map_or(n, |j| j + 2);
            span(&mut out, "com", &text(i, end));
            i = end;
            continue;
        }
        // Atributy Rustu.
        if l == Lang::Rust
            && c == '#'
            && (next == '[' || (next == '!' && chars.get(i + 2) == Some(&'[')))
        {
            let mut depth = 0;
            let mut j = i;
            while j < n {
                if chars[j] == '[' {
                    depth += 1;
                } else if chars[j] == ']' {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                j += 1;
            }
            span(&mut out, "attr", &text(i, j));
            i = j;
            continue;
        }
        // Retezce (s escape sekvencemi); b"..." v Rustu.
        if c == '"'
            || (l != Lang::Rust && c == '\'' && l != Lang::C)
            || (l == Lang::Rust && c == 'b' && next == '"')
        {
            let quote = if c == 'b' { '"' } else { c };
            let start = i;
            let mut j = if c == 'b' { i + 2 } else { i + 1 };
            while j < n && chars[j] != quote {
                // Escape sekvence: v "..." a vsude v Rustu/C; '...' v TOML a shellu je doslovny.
                if chars[j] == '\\' && (quote == '"' || matches!(l, Lang::Rust | Lang::C)) {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(n);
            span(&mut out, "str", &text(start, end));
            i = end;
            line_start = false;
            continue;
        }
        // Znaky a lifetimy v Rustu a C.
        if c == '\'' && matches!(l, Lang::Rust | Lang::C) {
            // Znak: 'x' nebo '\n'.
            let close = if next == '\\' { i + 3 } else { i + 2 };
            if chars.get(close) == Some(&'\'') {
                span(&mut out, "str", &text(i, close + 1));
                i = close + 1;
                continue;
            }
            if l == Lang::Rust && (next.is_alphabetic() || next == '_') {
                let mut j = i + 1;
                while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                span(&mut out, "life", &text(i, j));
                i = j;
                continue;
            }
        }
        // TOML: sekce [x] a klic na zacatku radku.
        if l == Lang::Toml && line_start && c == '[' {
            let end = chars[i..]
                .iter()
                .position(|&x| x == '\n')
                .map_or(n, |p| i + p);
            span(&mut out, "ty", &text(i, end));
            i = end;
            continue;
        }
        if l == Lang::Toml && line_start && (c.is_alphanumeric() || c == '_' || c == '-') {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || matches!(chars[j], '_' | '-' | '.')) {
                j += 1;
            }
            span(&mut out, "fn", &text(i, j));
            i = j;
            line_start = false;
            continue;
        }
        // Cisla.
        if c.is_ascii_digit()
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
        {
            let mut j = i;
            while j < n
                && (chars[j].is_ascii_alphanumeric()
                    || chars[j] == '_'
                    || (chars[j] == '.' && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                j += 1;
            }
            span(&mut out, "num", &text(i, j));
            i = j;
            line_start = false;
            continue;
        }
        // Slova.
        if c.is_alphabetic()
            || c == '_'
            || (l == Lang::Bash
                && c == '-'
                && next.is_alphabetic()
                && (i == 0 || chars[i - 1] == ' '))
        {
            let mut j = i + 1;
            while j < n
                && (chars[j].is_alphanumeric()
                    || chars[j] == '_'
                    || (l == Lang::Bash && chars[j] == '-'))
            {
                j += 1;
            }
            let word = text(i, j);
            let after = chars.get(j).copied().unwrap_or('\0');
            let class = match l {
                Lang::Rust if RUST_KW.contains(&word.as_str()) => Some("kw"),
                Lang::C if C_KW.contains(&word.as_str()) => Some("kw"),
                Lang::Bash if BASH_KW.contains(&word.as_str()) => Some("kw"),
                Lang::Bash if word.starts_with('-') => Some("attr"),
                Lang::Bash
                    if line_start
                        || chars[..i].iter().rev().take_while(|&&x| x == ' ').count() == i =>
                {
                    Some("fn")
                }
                Lang::Toml if word == "true" || word == "false" => Some("kw"),
                Lang::Rust if after == '!' => Some("mac"),
                Lang::Rust | Lang::C if RUST_PRIM.contains(&word.as_str()) && l == Lang::Rust => {
                    Some("ty")
                }
                Lang::Rust | Lang::C if word.chars().next().is_some_and(char::is_uppercase) => {
                    if word
                        .chars()
                        .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_')
                        && word.len() > 1
                    {
                        Some("const")
                    } else {
                        Some("ty")
                    }
                }
                Lang::Rust | Lang::C if after == '(' => Some("fn"),
                _ => None,
            };
            match class {
                Some("mac") => {
                    let mut w = word.clone();
                    w.push('!');
                    span(&mut out, "mac", &w);
                    j += 1;
                }
                Some(cl) => span(&mut out, cl, &word),
                None => esc(&mut out, &word),
            }
            i = j;
            line_start = false;
            continue;
        }
        if c == '\n' {
            line_start = true;
        } else if !c.is_whitespace() {
            line_start = false;
        }
        let mut buf = [0u8; 4];
        esc(&mut out, c.encode_utf8(&mut buf));
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::highlight;

    #[test]
    fn rust_tokens() {
        let h = highlight(
            "rust",
            "fn main() { let x: u32 = 5; println!(\"a<b\"); } // konec",
        );
        assert!(h.contains("<span class=\"hl-kw\">fn</span>"));
        assert!(h.contains("<span class=\"hl-fn\">main</span>"));
        assert!(h.contains("<span class=\"hl-ty\">u32</span>"));
        assert!(h.contains("<span class=\"hl-num\">5</span>"));
        assert!(h.contains("<span class=\"hl-mac\">println!</span>"));
        assert!(h.contains("<span class=\"hl-str\">&quot;a&lt;b&quot;</span>"));
        assert!(h.contains("<span class=\"hl-com\">// konec</span>"));
    }

    /// Dokumentace a terminal se nerozchazeji: tridy v HTML jsou presne
    /// useky `shlang::highlight` (stejna funkce kresli radek bloku v
    /// terminalu) pro vsechny priklady shellu v dokumentaci.
    #[test]
    fn shell_docs_match_terminal() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut checked = 0;
        for f in ["docs/cs/navrh-shell.md", "docs/en/shell-design.md"] {
            let md = std::fs::read_to_string(root.join(f)).unwrap();
            let mut code: Option<String> = None;
            for line in md.lines() {
                match &mut code {
                    None if line.starts_with("```shell-") || line == "```mos" => code = Some(String::new()),
                    Some(c) if line.starts_with("```") => {
                        let html = super::mos(c);
                        let classes: Vec<String> = html
                            .split("<span class=\"sh-")
                            .skip(1)
                            .map(|p| p.split('"').next().unwrap().to_string())
                            .collect();
                        let term: Vec<String> = shlang::highlight::highlight(c, None)
                            .spans
                            .iter()
                            .map(|s| s.class.name().to_string())
                            .collect();
                        assert_eq!(classes, term, "{f}: {c}");
                        checked += 1;
                        code = None;
                    }
                    Some(c) => {
                        c.push_str(line);
                        c.push('\n');
                    }
                    None => {}
                }
            }
        }
        assert!(checked > 10, "prikladu shellu: {checked}");
    }

    /// Terminal kresli kazdy usek barvou sve tridy z motivu a dokumentace
    /// ma pro tutez tridu tutez barvu (CSS z motivu): pro vsechny
    /// priklady shellu v dokumentaci, oba motivy.
    #[test]
    fn terminal_colors_match_docs_css() {
        use term::theme::Theme;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let md = std::fs::read_to_string(root.join("docs/cs/navrh-shell.md")).unwrap().replace("\r\n", "\n");
        let css = super::shell_css();
        for theme in [Theme::dark(), Theme::light()] {
            let mut checked = 0;
            for block in md.split("```shell-b\n").skip(1) {
                let code = block.split("```").next().unwrap();
                let h = shlang::highlight::highlight(code, None);
                let ansi = term::render::line(code, &h, &theme);
                for sp in &h.spans {
                    let st = theme.syntax(sp.class);
                    let want = format!("{}{}", term::render::sgr(st), &code[sp.start..sp.end]);
                    assert!(ansi.contains(&want), "{}: usek {:?} bez barvy sve tridy", theme.name, &code[sp.start..sp.end]);
                    let var = format!("--sh-{}: {};", sp.class.name(), st.fg.hex());
                    assert!(css.contains(&var), "CSS dokumentace nema barvu {var}");
                    checked += 1;
                }
            }
            assert!(checked > 50, "useku: {checked}");
        }
    }

    #[test]
    fn shell_css_has_every_class() {
        let css = super::shell_css();
        for c in shlang::highlight::Class::ALL {
            assert!(css.contains(&format!(".sh-{} ", c.name())), "{}", c.name());
            assert!(css.contains(&format!("--sh-{}:", c.name())));
        }
    }

    #[test]
    fn unknown_language_is_escaped_only() {
        assert_eq!(highlight("text", "a < b"), "a &lt; b");
    }

    #[test]
    fn text_is_preserved() {
        let src = "#[derive(Debug)]\nstruct A<'a> { x: &'a str, c: char = 'x' }\n/* blok */ b\"bytes\\n\"";
        let h = highlight("rust", src);
        let plain = h
            .split('<')
            .map(|p| p.split_once('>').map_or(p, |(_, t)| t))
            .collect::<String>()
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        assert_eq!(plain, src);
    }
}
