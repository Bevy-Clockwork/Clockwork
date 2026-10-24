//! Recipes as text: where every value sits in a RON file, so tools can change one number, one
//! name or one step and leave everything else (the agent's comments above all) as it was.
//!
//! [`Doc::parse`] reads the structure of any RON text without its meaning; [`Recipe`] values
//! still come from `ron` itself. Changes are made by replacing spans and parsing again.
//!
//! [`Recipe`]: clockwork_core::recipe::Recipe

use std::ops::Range;

/// A RON value and the bytes it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub span: Range<usize>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A number, string, char, `true`/`false` or a bare identifier (a unit enum variant).
    Scalar,
    /// `Name(field: value, …)` or `(field: value, …)`.
    Struct {
        name: Option<String>,
        fields: Vec<Field>,
    },
    /// `Name(value, …)`, `(value, …)` or `Name()`.
    Tuple {
        name: Option<String>,
        items: Vec<Node>,
    },
    /// `[value, …]`.
    Seq(Vec<Node>),
    /// `{key: value, …}`.
    Map(Vec<(Node, Node)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub value: Node,
}

pub use clockwork_core::recipe::Seg;

/// Parsed RON text.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub text: String,
    pub root: Node,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("at byte {at}: {message}")]
pub struct TextError {
    pub at: usize,
    pub message: String,
}

impl Doc {
    pub fn parse(text: impl Into<String>) -> Result<Doc, TextError> {
        let text = text.into();
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        p.skip_attributes()?;
        let root = p.value()?;
        p.ws()?;
        if p.i < p.s.len() {
            return Err(p.error("text after the value"));
        }
        Ok(Doc { text, root })
    }

    /// The text of a node.
    pub fn text_of(&self, node: &Node) -> &str {
        &self.text[node.span.clone()]
    }

    /// The text with `span` replaced.
    pub fn replaced(&self, span: Range<usize>, with: &str) -> String {
        let mut text = self.text.clone();
        text.replace_range(span, with);
        text
    }

    /// The text with `item` (written without a trailing comma) added at the end of the sequence
    /// `seq`, on its own line, indented like the items before it.
    pub fn appended(&self, seq: &Node, item: &str) -> String {
        let Kind::Seq(items) = &seq.kind else {
            return self.text.clone();
        };
        let mut text = self.text.clone();
        match items.last() {
            Some(last) => {
                let indent = indent_at(&self.text, last.span.start);
                let after = self.after_comma(last.span.end);
                let insert = if after.1 {
                    format!("\n{indent}{item},")
                } else {
                    format!(",\n{indent}{item},")
                };
                text.insert_str(after.0, &insert);
            }
            None => {
                let outer = indent_at(&self.text, seq.span.start);
                let insert = format!("\n{outer}    {item},\n{outer}");
                text.insert_str(seq.span.start + 1, &insert);
            }
        }
        text
    }

    /// The text with `item` added to the sequence `seq` so it becomes its `index`th element: on
    /// its own line above the element now there (below the comments above that element), or at
    /// the end.
    pub fn inserted(&self, seq: &Node, index: usize, item: &str) -> String {
        let Kind::Seq(items) = &seq.kind else {
            return self.text.clone();
        };
        let Some(before) = items.get(index) else {
            return self.appended(seq, item);
        };
        let start = before.span.start;
        let line_start = self.text[..start].rfind('\n').map_or(0, |i| i + 1);
        let mut text = self.text.clone();
        if self.text[line_start..start].trim().is_empty() {
            let indent = &self.text[line_start..start];
            text.insert_str(line_start, &format!("{indent}{item},\n"));
        } else {
            text.insert_str(start, &format!("{item}, "));
        }
        text
    }

    /// The text with field `key: value` added at the end of the struct `item` (for fields that
    /// were left out), on its own line when the struct's fields are.
    pub fn field_added(&self, item: &Node, key: &str, value: &str) -> Option<String> {
        let Kind::Struct { fields, .. } = &item.kind else {
            return None;
        };
        let mut text = self.text.clone();
        match fields.last() {
            Some(last) => {
                let (after, comma) = self.after_comma(last.value.span.end);
                let multiline = self.text[item.span.clone()].contains('\n');
                let insert = if multiline {
                    let indent = indent_at(&self.text, last.value.span.start);
                    format!("{}\n{indent}{key}: {value},", if comma { "" } else { "," })
                } else {
                    format!("{} {key}: {value}", if comma { "" } else { "," })
                };
                text.insert_str(after, &insert);
            }
            None => {
                let close = item.span.end - 1;
                text.insert_str(close, &format!("{key}: {value}"));
            }
        }
        Some(text)
    }

    /// The text with items `a` and `b` of a sequence (`a` before `b`) trading places. Each moves
    /// with the rest of its line (a comment after it); comment lines above stay where they are, as
    /// they often describe a group. `None` when either item shares its line with another.
    pub fn swapped(&self, a: &Node, b: &Node) -> Option<String> {
        let line = |item: &Node| -> Option<(Range<usize>, String)> {
            let line_start = self.text[..item.span.start]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            if !self.text[line_start..item.span.start].trim().is_empty() {
                return None;
            }
            let (after, comma) = self.after_comma(item.span.end);
            let rest = &self.text[after..];
            let line_len = rest.find('\n').map_or(rest.len(), |i| i + 1);
            let tail = rest[..line_len].trim();
            if !(tail.is_empty() || tail.starts_with("//") || tail.starts_with("/*")) {
                return None;
            }
            let range = line_start..after + line_len;
            let mut text = self.text[range.clone()].to_string();
            if !comma {
                text.insert(item.span.end - line_start, ',');
            }
            if !text.ends_with('\n') {
                text.push('\n');
            }
            Some((range, text))
        };
        let (first, first_text) = line(a)?;
        let (second, second_text) = line(b)?;
        if first.end > second.start {
            return None;
        }
        let mut text = String::new();
        text.push_str(&self.text[..first.start]);
        text.push_str(&second_text);
        text.push_str(&self.text[first.end..second.start]);
        text.push_str(&first_text);
        text.push_str(&self.text[second.end..]);
        Some(text)
    }

    /// The note on `item` (an element of a sequence written on its own line): the `//` comment
    /// lines right above it, up to a blank line or a section header (`// ==`, `// --`, `// ##`),
    /// without their slashes; and the span of those lines (empty, at the item's line, when there
    /// are none).
    pub fn note(&self, item: &Node) -> (Range<usize>, String) {
        let line_start = self.text[..item.span.start]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let mut start = line_start;
        let mut lines = Vec::new();
        while start > 0 {
            let above = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
            let line = self.text[above..start - 1].trim();
            let Some(comment) = line.strip_prefix("//") else {
                break;
            };
            let comment = comment.trim();
            if ["==", "--", "##"].iter().any(|m| comment.starts_with(m)) {
                break;
            }
            lines.push(comment.to_string());
            start = above;
        }
        lines.reverse();
        (start..line_start, lines.join("\n"))
    }

    /// The text with the note on `item` (see [`Doc::note`]) replaced by `note`, written as `//`
    /// lines indented like the item; an empty note takes the lines away.
    pub fn with_note(&self, item: &Node, note: &str) -> String {
        let (span, _) = self.note(item);
        let indent = indent_at(&self.text, item.span.start);
        let lines: String = note
            .lines()
            .map(str::trim_end)
            .skip_while(|l| l.trim().is_empty())
            .map(|l| {
                if l.trim().is_empty() {
                    format!("{indent}//\n")
                } else {
                    format!("{indent}// {l}\n")
                }
            })
            .collect();
        self.replaced(span, &lines)
    }

    /// The text without `item` (an element of a sequence): its line goes too when nothing else
    /// is on it, and so does its comma.
    pub fn removed(&self, item: &Node) -> String {
        let bytes = self.text.as_bytes();
        let (end, _) = self.after_comma(item.span.end);
        let mut start = item.span.start;
        let line_start = self.text[..start].rfind('\n').map_or(0, |i| i + 1);
        let mut end = end;
        if self.text[line_start..start].trim().is_empty() {
            let rest = &self.text[end..];
            let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
            if rest[..line_end].trim().is_empty() {
                start = line_start;
                end += line_end;
            }
        } else {
            // Something before it on the line: keep a space between neighbours.
            while start > line_start && bytes[start - 1] == b' ' {
                start -= 1;
            }
        }
        let mut text = self.text.clone();
        text.replace_range(start..end, "");
        text
    }

    /// Where a value ending at `end` is followed by its comma (after it, if there is one, and
    /// whether there is).
    fn after_comma(&self, end: usize) -> (usize, bool) {
        let mut p = Parser {
            s: self.text.as_bytes(),
            i: end,
        };
        if p.ws().is_ok() && p.peek() == Some(b',') {
            (p.i + 1, true)
        } else {
            (end, false)
        }
    }
}

fn indent_at(text: &str, at: usize) -> String {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..at]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect()
}

impl Node {
    /// A struct field's value.
    pub fn field(&self, key: &str) -> Option<&Node> {
        match &self.kind {
            Kind::Struct { fields, .. } => fields.iter().find(|f| f.key == key).map(|f| &f.value),
            _ => None,
        }
    }

    /// The variant or struct name, if written.
    pub fn name(&self) -> Option<&str> {
        match &self.kind {
            Kind::Struct { name, .. } | Kind::Tuple { name, .. } => name.as_deref(),
            _ => None,
        }
    }

    /// `Some(x)` is `x`; everything else is itself.
    pub fn unwrap_some(&self) -> &Node {
        match &self.kind {
            Kind::Tuple { name, items } if name.as_deref() == Some("Some") && items.len() == 1 => {
                &items[0]
            }
            _ => self,
        }
    }

    pub fn items(&self) -> &[Node] {
        match &self.kind {
            Kind::Seq(items) => items,
            _ => &[],
        }
    }

    /// The node at `path`, looking through `Some(…)`.
    pub fn at(&self, path: &[Seg], text: &str) -> Option<&Node> {
        let mut node = self;
        for seg in path {
            let here = node.unwrap_some();
            node = match seg {
                Seg::Field(key) => here.field(key)?,
                Seg::Index(i) => here.items().get(*i)?,
                Seg::Key(k) => match &here.kind {
                    Kind::Map(entries) => entries
                        .iter()
                        .find(|(key, _)| parse_int(&text[key.span.clone()]) == Some(*k as i64))
                        .map(|(_, v)| v)?,
                    _ => return None,
                },
            };
        }
        Some(node)
    }
}

/// An integer literal (`12`, `-3`, `0x1F`, `0b101`, `0o7`, with `_` separators).
pub fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim().replace('_', "");
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, text.strip_prefix('+').unwrap_or(&text).to_string()),
    };
    let lower = digits.to_ascii_lowercase();
    let value = if let Some(hex) = lower.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(bin) = lower.strip_prefix("0b") {
        i64::from_str_radix(bin, 2).ok()?
    } else if let Some(oct) = lower.strip_prefix("0o") {
        i64::from_str_radix(oct, 8).ok()?
    } else {
        lower.parse().ok()?
    };
    Some(if negative { -value } else { value })
}

/// `value` written like `like` was: hexadecimal stays hexadecimal (with its digits' case).
pub fn format_int_like(value: i64, like: &str) -> String {
    let like = like.trim();
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    if like
        .trim_start_matches('-')
        .to_ascii_lowercase()
        .starts_with("0x")
    {
        let digits = &like.trim_start_matches('-')[2..];
        if digits.chars().any(|c| c.is_ascii_lowercase()) {
            format!("{sign}0x{magnitude:x}")
        } else {
            format!("{sign}0x{magnitude:X}")
        }
    } else {
        value.to_string()
    }
}

/// The string a string literal stands for.
pub fn parse_string(text: &str) -> Option<String> {
    ron::from_str(text).ok()
}

/// A string as a RON literal.
pub fn string_literal(value: &str) -> String {
    ron::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> TextError {
        TextError {
            at: self.i,
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Skips whitespace and comments.
    fn ws(&mut self) -> Result<(), TextError> {
        loop {
            match (self.peek(), self.s.get(self.i + 1).copied()) {
                (Some(c), _) if c.is_ascii_whitespace() => self.i += 1,
                (Some(b'/'), Some(b'/')) => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    let start = self.i;
                    self.i += 2;
                    let mut depth = 1;
                    while depth > 0 {
                        match (self.peek(), self.s.get(self.i + 1).copied()) {
                            (None, _) => {
                                self.i = start;
                                return Err(self.error("unclosed comment"));
                            }
                            (Some(b'/'), Some(b'*')) => {
                                depth += 1;
                                self.i += 2;
                            }
                            (Some(b'*'), Some(b'/')) => {
                                depth -= 1;
                                self.i += 2;
                            }
                            _ => self.i += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// Skips `#![enable(…)]` lines.
    fn skip_attributes(&mut self) -> Result<(), TextError> {
        loop {
            self.ws()?;
            if self.s[self.i..].starts_with(b"#!") {
                while self.peek().is_some_and(|c| c != b']') {
                    self.i += 1;
                }
                self.i += 1;
            } else {
                return Ok(());
            }
        }
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        if self.s[self.i..].starts_with(b"r#") {
            self.i += 2;
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.i += 1;
        }
        if self.i == start || self.s[start].is_ascii_digit() {
            self.i = start;
            return None;
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn value(&mut self) -> Result<Node, TextError> {
        self.ws()?;
        let start = self.i;
        let Some(c) = self.peek() else {
            return Err(self.error("expected a value"));
        };
        let kind = match c {
            b'"' => {
                self.string()?;
                Kind::Scalar
            }
            b'\'' => {
                self.i += 1;
                if self.peek() == Some(b'\\') {
                    self.i += 1;
                }
                self.i += 1;
                while self.peek().is_some_and(|c| c != b'\'') {
                    self.i += 1;
                }
                self.i += 1;
                Kind::Scalar
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws()?;
                    match self.peek() {
                        Some(b']') => break,
                        None => return Err(self.error("unclosed [")),
                        _ => {}
                    }
                    items.push(self.value()?);
                    self.ws()?;
                    if self.peek() == Some(b',') {
                        self.i += 1;
                    }
                }
                self.i += 1;
                Kind::Seq(items)
            }
            b'{' => {
                self.i += 1;
                let mut entries = Vec::new();
                loop {
                    self.ws()?;
                    match self.peek() {
                        Some(b'}') => break,
                        None => return Err(self.error("unclosed {")),
                        _ => {}
                    }
                    let key = self.value()?;
                    self.ws()?;
                    if self.peek() != Some(b':') {
                        return Err(self.error("expected : in a map"));
                    }
                    self.i += 1;
                    let value = self.value()?;
                    entries.push((key, value));
                    self.ws()?;
                    if self.peek() == Some(b',') {
                        self.i += 1;
                    }
                }
                self.i += 1;
                Kind::Map(entries)
            }
            b'(' => self.parens(None)?,
            b'r' if self.s[self.i..].starts_with(b"r\"")
                || self.s[self.i..].starts_with(b"r#\"") =>
            {
                self.raw_string()?;
                Kind::Scalar
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let name = self.ident().ok_or_else(|| self.error("expected a name"))?;
                let before = self.i;
                self.ws()?;
                if self.peek() == Some(b'(') {
                    self.parens(Some(name))?
                } else {
                    self.i = before;
                    Kind::Scalar
                }
            }
            c if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.') => {
                self.i += 1;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.'))
                    || (matches!(self.peek(), Some(b'-' | b'+'))
                        && matches!(self.s[self.i - 1], b'e' | b'E')
                        && !self.s[start..self.i].starts_with(b"0x"))
                {
                    self.i += 1;
                }
                Kind::Scalar
            }
            _ => return Err(self.error("unexpected character")),
        };
        Ok(Node {
            span: start..self.i,
            kind,
        })
    }

    /// `( … )` after an optional name: a struct if it starts with `field:`, else a tuple.
    fn parens(&mut self, name: Option<String>) -> Result<Kind, TextError> {
        self.i += 1;
        self.ws()?;
        let is_struct = {
            let save = self.i;
            let named = self.ident().is_some() && {
                self.ws()?;
                self.peek() == Some(b':')
            };
            self.i = save;
            named
        };
        let mut fields = Vec::new();
        let mut items = Vec::new();
        loop {
            self.ws()?;
            match self.peek() {
                Some(b')') => break,
                None => return Err(self.error("unclosed (")),
                _ => {}
            }
            if is_struct {
                let key = self
                    .ident()
                    .ok_or_else(|| self.error("expected a field name"))?;
                self.ws()?;
                if self.peek() != Some(b':') {
                    return Err(self.error("expected : after a field name"));
                }
                self.i += 1;
                let value = self.value()?;
                fields.push(Field {
                    key: key.trim_start_matches("r#").to_string(),
                    value,
                });
            } else {
                items.push(self.value()?);
            }
            self.ws()?;
            if self.peek() == Some(b',') {
                self.i += 1;
            }
        }
        self.i += 1;
        Ok(if is_struct {
            Kind::Struct { name, fields }
        } else {
            Kind::Tuple { name, items }
        })
    }

    fn string(&mut self) -> Result<(), TextError> {
        let start = self.i;
        self.i += 1;
        loop {
            match self.peek() {
                None => {
                    self.i = start;
                    return Err(self.error("unclosed string"));
                }
                Some(b'\\') => self.i += 2,
                Some(b'"') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => self.i += 1,
            }
        }
    }

    fn raw_string(&mut self) -> Result<(), TextError> {
        let start = self.i;
        self.i += 1;
        let mut hashes = 0;
        while self.peek() == Some(b'#') {
            hashes += 1;
            self.i += 1;
        }
        self.i += 1;
        let close: Vec<u8> = std::iter::once(b'"')
            .chain(std::iter::repeat_n(b'#', hashes))
            .collect();
        while !self.s[self.i..].starts_with(&close) {
            if self.i >= self.s.len() {
                self.i = start;
                return Err(self.error("unclosed raw string"));
            }
            self.i += 1;
        }
        self.i += close.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_note_is_the_comment_right_above_an_item() {
        let text = "(steps: [\n    // == Tiles ==\n    // From the gfx table.\n    // Compressed.\n    A(x: 1),\n\n    B(x: 2),\n])";
        let doc = Doc::parse(text).unwrap();
        let items = doc.root.field("steps").unwrap().items();
        assert_eq!(doc.note(&items[0]).1, "From the gfx table.\nCompressed.");
        assert_eq!(doc.note(&items[1]).1, "");

        let changed = doc.with_note(&items[0], "One line");
        assert!(changed.contains("// == Tiles ==\n    // One line\n    A(x: 1)"));
        let doc = Doc::parse(changed).unwrap();
        let items = doc.root.field("steps").unwrap().items();
        let changed = doc.with_note(&items[1], "New\nnote");
        assert!(changed.contains("\n    // New\n    // note\n    B(x: 2)"));
        let doc = Doc::parse(changed).unwrap();
        let items = doc.root.field("steps").unwrap().items();
        let gone = doc.with_note(&items[0], "");
        assert!(gone.contains("// == Tiles ==\n    A(x: 1)"));
    }

    use super::*;
    use clockwork_core::recipe::Recipe;

    const RECIPE: &str = r#"// A recipe, with comments that must survive.
Recipe(
    steps: [
        // The tiles.
        Read(name: "tiles", offset: 0x324EB4, codec: GbaLz77),
        Read(name: "pal", offset: 0x10, size: Some(32)), /* trailing */
        GbaPalettes(from: "pal", bank: "bg", slot: 0),
    ],
    outputs: [
        Image(id: "sheet", from: "sheet", kind: Some(Tileset)),
        Surfaces(id: "s", width: 2, cell_size: 16, grids: [
            (name: "g", from: "codes", codes: {0x01: [Solid], 2: []}),
        ]),
    ],
)
"#;

    fn doc() -> Doc {
        Doc::parse(RECIPE).unwrap()
    }

    fn steps(doc: &Doc) -> &Node {
        doc.root.field("steps").unwrap()
    }

    #[test]
    fn finds_every_value() {
        let doc = doc();
        assert_eq!(doc.root.name(), Some("Recipe"));
        let read = &steps(&doc).items()[0];
        assert_eq!(read.name(), Some("Read"));
        assert_eq!(doc.text_of(read.field("offset").unwrap()), "0x324EB4");
        let size = steps(&doc).items()[1].field("size").unwrap();
        assert_eq!(doc.text_of(size.unwrap_some()), "32");
        let grid_from = doc.root.at(
            &[
                Seg::Field("outputs"),
                Seg::Index(1),
                Seg::Field("grids"),
                Seg::Index(0),
                Seg::Field("from"),
            ],
            &doc.text,
        );
        assert_eq!(doc.text_of(grid_from.unwrap()), "\"codes\"");
        let code = doc.root.at(
            &[
                Seg::Field("outputs"),
                Seg::Index(1),
                Seg::Field("grids"),
                Seg::Index(0),
                Seg::Field("codes"),
                Seg::Key(1),
            ],
            &doc.text,
        );
        assert_eq!(doc.text_of(code.unwrap()), "[Solid]");
    }

    #[test]
    fn replacing_a_value_keeps_the_comments() {
        let doc = doc();
        let offset = steps(&doc).items()[0].field("offset").unwrap();
        let text = doc.replaced(
            offset.span.clone(),
            &format_int_like(0x324EC0, doc.text_of(offset)),
        );
        assert!(text.contains("offset: 0x324EC0"));
        assert!(text.contains("// The tiles.") && text.contains("/* trailing */"));
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps.len(), 3);
    }

    #[test]
    fn steps_are_appended_and_removed_line_by_line() {
        let doc = doc();
        let text = doc.appended(steps(&doc), r#"Fill(name: "blank", size: 4)"#);
        assert!(text.contains("        Fill(name: \"blank\", size: 4),\n    ],"));
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps.len(), 4);

        let doc = Doc::parse(text).unwrap();
        let text = doc.removed(&steps(&doc).items()[2]);
        assert!(!text.contains("GbaPalettes"));
        assert!(text.contains("/* trailing */\n        Fill"));
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps.len(), 3);
    }

    #[test]
    fn neighbours_swap_with_their_line_comments() {
        let doc = doc();
        let items = steps(&doc).items();
        let text = doc.swapped(&items[1], &items[2]).unwrap();
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert!(matches!(
            recipe.steps[1],
            clockwork_core::recipe::Step::GbaPalettes { .. }
        ));
        assert!(text.contains(
            "slot: 0),\n        Read(name: \"pal\", offset: 0x10, size: Some(32)), /* trailing */"
        ));
        assert!(text.contains("// The tiles.\n        Read(name: \"tiles\""));

        // The last item gets the comma it lacked.
        let doc = Doc::parse(
            "Recipe(steps: [\n    Fill(name: \"a\", size: 1),\n    Fill(name: \"b\", size: 2)\n], outputs: [])",
        )
        .unwrap();
        let items = doc.root.field("steps").unwrap().items();
        let text = doc.swapped(&items[0], &items[1]).unwrap();
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert!(matches!(
            &recipe.steps[0],
            clockwork_core::recipe::Step::Fill { size: 2, .. }
        ));

        let doc = Doc::parse("[1, 2]").unwrap();
        let items = doc.root.items();
        assert_eq!(
            doc.swapped(&items[0], &items[1]),
            None,
            "items on one line don't move"
        );
    }

    #[test]
    fn appending_to_an_empty_list() {
        let doc = Doc::parse("Recipe(steps: [], outputs: [])").unwrap();
        let text = doc.appended(
            doc.root.field("steps").unwrap(),
            r#"Fill(name: "a", size: 1)"#,
        );
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps.len(), 1);
    }

    #[test]
    fn numbers_keep_how_they_were_written() {
        assert_eq!(parse_int("0x1F"), Some(31));
        assert_eq!(parse_int("-0x10"), Some(-16));
        assert_eq!(parse_int("1_000"), Some(1000));
        assert_eq!(format_int_like(255, "0x10"), "0xFF");
        assert_eq!(format_int_like(255, "0xa"), "0xff");
        assert_eq!(format_int_like(7, "3"), "7");
        assert_eq!(string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(parse_string("\"vram\"").as_deref(), Some("vram"));
    }

    #[test]
    fn every_saved_shape_parses() {
        for text in [
            "#![enable(implicit_some)]\n(a: 1)",
            "Unit",
            "(1, 2.5e-3, 'c', r#\"raw\"#)",
            "Foo(bar: [], baz: {})",
        ] {
            Doc::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        }
        assert!(Doc::parse("(a: 1").is_err());
    }

    #[test]
    fn steps_go_in_at_any_place_and_fields_are_added() {
        let doc = doc();
        let text = doc.inserted(steps(&doc), 0, r#"Fill(name: "a", size: 1)"#);
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps[0].name(), "Fill");
        assert!(text.contains("// The tiles.\n        Fill("), "{text}");
        let text = doc.inserted(steps(&doc), 2, r#"Fill(name: "a", size: 1)"#);
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps[2].name(), "Fill");
        assert!(text.contains("/* trailing */\n        Fill("), "{text}");
        let text = doc.inserted(steps(&doc), 3, r#"Fill(name: "a", size: 1)"#);
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert_eq!(recipe.steps[3].name(), "Fill");

        let read = &steps(&doc).items()[0];
        let text = doc.field_added(read, "size", "Some(4)").unwrap();
        let recipe: Recipe = ron::from_str(&text).unwrap();
        assert!(text.contains("codec: GbaLz77, size: Some(4))"), "{text}");
        assert!(matches!(
            recipe.steps[0],
            clockwork_core::recipe::Step::Read { size: Some(4), .. }
        ));
        let multi = Doc::parse("Recipe(\n    steps: [],\n    outputs: [],\n)").unwrap();
        let text = multi.field_added(&multi.root, "extra", "1").unwrap();
        assert_eq!(
            text,
            "Recipe(\n    steps: [],\n    outputs: [],\n    extra: 1,\n)"
        );
    }
}
