//! Minimalni parser XML pro registr Vulkanu (`vk.xml` a XML Venusu).
//!
//! Umi jen to, co registr pouziva: prvky s atributy, text, komentare,
//! deklaraci `<?xml ... ?>` a entity `&lt; &gt; &amp; &quot; &apos; &#N;`.
//! CDATA, DOCTYPE a jmenne prostory ne (registr je nema). Chyba = text
//! s pozici v souboru.

/// Uzel stromu.
#[derive(Debug, Clone)]
pub enum Node {
    Elem(Element),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Primi potomci s danym jmenem.
    pub fn elems<'a, 'n>(&'a self, name: &'n str) -> impl Iterator<Item = &'a Element> + use<'a, 'n> {
        self.children.iter().filter_map(move |c| match c {
            Node::Elem(e) if e.name == name => Some(e),
            _ => None,
        })
    }

    /// Prvni primy potomek s danym jmenem.
    pub fn elem(&self, name: &str) -> Option<&Element> {
        self.elems(name).next()
    }

    /// Cely text prvku a jeho potomku (bez prvku `skip`, napr. `comment`).
    pub fn text_without(&self, skip: &str) -> String {
        let mut s = String::new();
        self.collect_text(skip, &mut s);
        s
    }

    fn collect_text(&self, skip: &str, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Elem(e) if e.name != skip => e.collect_text(skip, out),
                Node::Elem(_) => {}
            }
        }
    }

    /// Text prvniho potomka `name`.
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.elem(name).map(|e| e.text_without(""))
    }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

/// Rozebere dokument a vrati korenovy prvek.
pub fn parse(text: &str) -> Result<Element, String> {
    let mut p = Parser { s: text.as_bytes(), pos: 0 };
    p.skip_misc()?;
    let root = p.element()?;
    p.skip_misc()?;
    if p.pos != p.s.len() {
        return Err(p.err("data za korenovym prvkem"));
    }
    Ok(root)
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        let line = self.s[..self.pos.min(self.s.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        format!("XML radek {line}: {what}")
    }

    fn starts(&self, pat: &str) -> bool {
        self.s[self.pos..].starts_with(pat.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn skip_until(&mut self, end: &str) -> Result<(), String> {
        while self.pos < self.s.len() {
            if self.starts(end) {
                self.pos += end.len();
                return Ok(());
            }
            self.pos += 1;
        }
        Err(self.err(&format!("chybi {end}")))
    }

    /// Bile znaky, komentare a deklarace pred a za korenem.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.skip_until("?>")?;
            } else if self.starts("<!--") {
                self.skip_until("-->")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':' | b'.') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(self.err("ocekavano jmeno"));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
    }

    fn element(&mut self) -> Result<Element, String> {
        if !self.starts("<") {
            return Err(self.err("ocekavan prvek"));
        }
        self.pos += 1;
        let mut e = Element { name: self.name()?, ..Default::default() };
        loop {
            self.skip_ws();
            if self.starts("/>") {
                self.pos += 2;
                return Ok(e);
            }
            if self.starts(">") {
                self.pos += 1;
                break;
            }
            let k = self.name()?;
            self.skip_ws();
            if !self.starts("=") {
                return Err(self.err("ocekavano ="));
            }
            self.pos += 1;
            self.skip_ws();
            let q = *self.s.get(self.pos).ok_or_else(|| self.err("konec v atributu"))?;
            if q != b'"' && q != b'\'' {
                return Err(self.err("hodnota atributu bez uvozovek"));
            }
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.s.len() && self.s[self.pos] != q {
                self.pos += 1;
            }
            if self.pos >= self.s.len() {
                return Err(self.err("neukonceny atribut"));
            }
            let raw = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
            self.pos += 1;
            e.attrs.push((k, unescape(&raw).map_err(|m| self.err(&m))?));
        }
        // Obsah prvku.
        loop {
            if self.pos >= self.s.len() {
                return Err(self.err(&format!("neukonceny prvek <{}>", e.name)));
            }
            if self.starts("</") {
                self.pos += 2;
                let n = self.name()?;
                if n != e.name {
                    return Err(self.err(&format!("</{n}> misto </{}>", e.name)));
                }
                self.skip_ws();
                if !self.starts(">") {
                    return Err(self.err("ocekavano >"));
                }
                self.pos += 1;
                return Ok(e);
            }
            if self.starts("<!--") {
                self.skip_until("-->")?;
            } else if self.starts("<![CDATA[") || self.starts("<!") || self.starts("<?") {
                return Err(self.err("CDATA, DOCTYPE ani instrukce uvnitr nejsou podporovane"));
            } else if self.starts("<") {
                let c = self.element()?;
                e.children.push(Node::Elem(c));
            } else {
                let start = self.pos;
                while self.pos < self.s.len() && self.s[self.pos] != b'<' {
                    self.pos += 1;
                }
                let raw = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
                e.children.push(Node::Text(unescape(&raw).map_err(|m| self.err(&m))?));
            }
        }
    }
}

/// Nahradi entity znaky.
fn unescape(s: &str) -> Result<String, String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(';').ok_or("neukoncena entita")?;
        let ent = &tail[1..end];
        let c = match ent {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ if ent.starts_with("#x") => {
                char::from_u32(u32::from_str_radix(&ent[2..], 16).map_err(|_| "neplatna entita")?).ok_or("neplatny znak")?
            }
            _ if ent.starts_with('#') => {
                char::from_u32(ent[1..].parse::<u32>().map_err(|_| "neplatna entita")?).ok_or("neplatny znak")?
            }
            _ => return Err(format!("neznama entita &{ent};")),
        };
        out.push(c);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_shapes() {
        let doc = r#"<?xml version="1.0"?>
<!-- komentar -->
<registry>
  <type category="struct" name="A" returnedonly='true'>
    <member len="null-terminated"><type>char</type> <name>x</name>[<enum>N</enum>]<comment>c</comment></member>
    <member>const <type>void</type>* <name>pNext</name></member>
  </type>
  <enum value="&quot;s&quot;" name="E"/>
</registry>"#;
        let r = parse(doc).unwrap();
        assert_eq!(r.name, "registry");
        let t = r.elem("type").unwrap();
        assert_eq!(t.attr("returnedonly"), Some("true"));
        let m: Vec<_> = t.elems("member").collect();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].text_without("comment"), "char x[N]");
        assert_eq!(m[0].child_text("name").as_deref(), Some("x"));
        assert_eq!(m[1].text_without("comment"), "const void* pNext");
        assert_eq!(r.elem("enum").unwrap().attr("value"), Some("\"s\""));
    }

    #[test]
    fn rejects_broken() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a x=1/>").is_err());
        assert!(parse("<a>&foo;</a>").is_err());
        assert!(parse("<a/><b/>").is_err());
    }
}
