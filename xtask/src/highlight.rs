//! Zvyrazneni syntaxe v dokumentaci (pri generovani HTML, bez skriptu v
//! prohlizeci a bez zavislosti).
//!
//! Jednoduchy lexer: komentare, retezce a znaky, cisla, klicova slova,
//! typy (jmena s velkym pocatecnim pismenem), makra (`jmeno!`), atributy
//! (`#[...]`), lifetimy (`'a`). Podporovane jazyky: `rust`, `c`, `cpp`,
//! `toml`, `bash`/`sh`; ostatni (`text`, bez jazyka) zustavaji bez barev.
//! Vystup jsou `<span class="hl-...">`, barvy definuje `style.css`.

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
const MOS_KW: &[&str] = &[
    "let", "fn", "if", "else", "for", "in", "while", "return", "break", "continue", "try", "bg", "true",
    "false", "null", "lang", "needs",
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
        "shell-b" | "shell-c" => Some(Lang::Mos),
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

/// Zvyrazni `code` v jazyce `lang`; vraci HTML (bez obalu `<pre>`).
pub fn highlight(lang: &str, code: &str) -> String {
    let mut out = String::with_capacity(code.len() * 2);
    let Some(l) = lang_of(lang) else {
        esc(&mut out, code);
        return out;
    };
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
            Lang::Toml | Lang::Bash | Lang::Mos => c == '#',
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
            || ((l == Lang::Bash || l == Lang::Mos)
                && c == '-'
                && next.is_alphabetic()
                && (i == 0 || chars[i - 1] == ' '))
        {
            let mut j = i + 1;
            while j < n
                && (chars[j].is_alphanumeric()
                    || chars[j] == '_'
                    || ((l == Lang::Bash || l == Lang::Mos) && chars[j] == '-'))
            {
                j += 1;
            }
            let word = text(i, j);
            let after = chars.get(j).copied().unwrap_or('\0');
            let class = match l {
                Lang::Rust if RUST_KW.contains(&word.as_str()) => Some("kw"),
                Lang::C if C_KW.contains(&word.as_str()) => Some("kw"),
                Lang::Bash if BASH_KW.contains(&word.as_str()) => Some("kw"),
                Lang::Mos if MOS_KW.contains(&word.as_str()) => Some("kw"),
                Lang::Mos if word.starts_with('-') => Some("attr"),
                Lang::Mos if after == '(' => Some("fn"),
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
