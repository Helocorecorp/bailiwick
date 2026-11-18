//! A tiny JSON value, a deterministic pretty-printer, a compact printer,
//! and a strict parser. Kept in-tree so the reference toolchain has no
//! third-party dependencies. Output is canonical: fixed key order (as
//! built), two-space indentation, scalar arrays on one line, integers
//! printed without a decimal point.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }
    pub fn opt_str(s: &Option<String>) -> Json {
        match s {
            Some(v) => Json::Str(v.clone()),
            None => Json::Null,
        }
    }
    pub fn opt_u8(n: Option<u8>) -> Json {
        match n {
            Some(v) => Json::Num(v as f64),
            None => Json::Null,
        }
    }
    pub fn opt_i64(n: Option<i64>) -> Json {
        match n {
            Some(v) => Json::Num(v as f64),
            None => Json::Null,
        }
    }
    pub fn opt_f64(n: Option<f64>) -> Json {
        match n {
            Some(v) => Json::Num(v),
            None => Json::Null,
        }
    }
    pub fn opt_bool(b: Option<bool>) -> Json {
        match b {
            Some(v) => Json::Bool(v),
            None => Json::Null,
        }
    }
    pub fn strs<S: AsRef<str>>(v: &[S]) -> Json {
        Json::Arr(v.iter().map(|s| Json::Str(s.as_ref().to_string())).collect())
    }
    pub fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    // --- accessors (for reading parsed JSON, e.g. LSP requests) ---------

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(o) => Some(o),
            _ => None,
        }
    }

    fn is_scalar(&self) -> bool {
        !matches!(self, Json::Arr(_) | Json::Obj(_))
    }
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn num(n: f64) -> String {
    if !n.is_finite() {
        return "null".to_string();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

fn write(j: &Json, indent: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n);
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(&num(*n)),
        Json::Str(s) => escape(s, out),
        Json::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else if items.iter().all(|i| i.is_scalar()) {
                out.push('[');
                for (k, it) in items.iter().enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    write(it, indent, out);
                }
                out.push(']');
            } else {
                out.push_str("[\n");
                for (k, it) in items.iter().enumerate() {
                    out.push_str(&pad(indent + 2));
                    write(it, indent + 2, out);
                    if k + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(indent));
                out.push(']');
            }
        }
        Json::Obj(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{\n");
                for (k, (key, val)) in fields.iter().enumerate() {
                    out.push_str(&pad(indent + 2));
                    escape(key, out);
                    out.push_str(": ");
                    write(val, indent + 2, out);
                    if k + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(indent));
                out.push('}');
            }
        }
    }
}

fn write_compact(j: &Json, out: &mut String) {
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(&num(*n)),
        Json::Str(s) => escape(s, out),
        Json::Arr(items) => {
            out.push('[');
            for (k, it) in items.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                write_compact(it, out);
            }
            out.push(']');
        }
        Json::Obj(fields) => {
            out.push('{');
            for (k, (key, val)) in fields.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                escape(key, out);
                out.push(':');
                write_compact(val, out);
            }
            out.push('}');
        }
    }
}

/// Serialize with a trailing newline.
pub fn to_string_pretty(j: &Json) -> String {
    let mut out = String::new();
    write(j, 0, &mut out);
    out.push('\n');
    out
}

/// Serialize on one line, no trailing newline (for JSON-RPC framing).
pub fn to_string_compact(j: &Json) -> String {
    let mut out = String::new();
    write_compact(j, &mut out);
    out
}

// --- parser -------------------------------------------------------------------

const MAX_DEPTH: usize = 256;

struct Parser {
    c: Vec<char>,
    p: usize,
    depth: usize,
}

impl Parser {
    fn err(&self, m: &str) -> String {
        format!("json: {} at offset {}", m, self.p)
    }
    fn peek(&self) -> Option<char> {
        self.c.get(self.p).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let r = self.peek();
        if r.is_some() {
            self.p += 1;
        }
        r
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\n') | Some('\r')) {
            self.p += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        if self.depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some('{') => {
                self.depth += 1;
                self.bump();
                let mut fields = Vec::new();
                self.skip_ws();
                if self.peek() == Some('}') {
                    self.bump();
                    self.depth -= 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.skip_ws();
                    if self.peek() != Some('"') {
                        return Err(self.err("expected a string key"));
                    }
                    let k = self.string()?;
                    self.skip_ws();
                    if self.bump() != Some(':') {
                        return Err(self.err("expected `:`"));
                    }
                    self.skip_ws();
                    let v = self.value()?;
                    fields.push((k, v));
                    self.skip_ws();
                    match self.bump() {
                        Some(',') => continue,
                        Some('}') => break,
                        _ => return Err(self.err("expected `,` or `}`")),
                    }
                }
                self.depth -= 1;
                Ok(Json::Obj(fields))
            }
            Some('[') => {
                self.depth += 1;
                self.bump();
                let mut items = Vec::new();
                self.skip_ws();
                if self.peek() == Some(']') {
                    self.bump();
                    self.depth -= 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.skip_ws();
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.bump() {
                        Some(',') => continue,
                        Some(']') => break,
                        _ => return Err(self.err("expected `,` or `]`")),
                    }
                }
                self.depth -= 1;
                Ok(Json::Arr(items))
            }
            Some('"') => Ok(Json::Str(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => {
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() || "+-.eE".contains(c) {
                        s.push(c);
                        self.p += 1;
                    } else {
                        break;
                    }
                }
                s.parse::<f64>()
                    .map(Json::Num)
                    .map_err(|_| self.err(&format!("invalid number `{}`", s)))
            }
            Some(c) => Err(self.err(&format!("unexpected character `{}`", c))),
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, String> {
        for ch in word.chars() {
            if self.bump() != Some(ch) {
                return Err(self.err("invalid literal"));
            }
        }
        Ok(v)
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.bump().ok_or_else(|| self.err("bad \\u escape"))?;
            let d = c.to_digit(16).ok_or_else(|| self.err("bad \\u escape"))?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.bump(); // opening quote
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string")),
                Some('"') => return Ok(s),
                Some('\\') => match self.bump() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('b') => s.push('\u{8}'),
                    Some('f') => s.push('\u{c}'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let cp = self.hex4()?;
                        if (0xD800..0xDC00).contains(&cp) {
                            if self.bump() != Some('\\') || self.bump() != Some('u') {
                                return Err(self.err("unpaired surrogate"));
                            }
                            let lo = self.hex4()?;
                            if !(0xDC00..0xE000).contains(&lo) {
                                return Err(self.err("invalid low surrogate"));
                            }
                            let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            s.push(char::from_u32(c).ok_or_else(|| self.err("invalid code point"))?);
                        } else {
                            s.push(char::from_u32(cp).ok_or_else(|| self.err("invalid code point"))?);
                        }
                    }
                    Some(o) => return Err(self.err(&format!("invalid escape `\\{}`", o))),
                    None => return Err(self.err("unterminated string")),
                },
                Some(c) => s.push(c),
            }
        }
    }
}

/// Parse a JSON document. Strict: no comments, no trailing commas.
pub fn parse(src: &str) -> Result<Json, String> {
    let mut p = Parser { c: src.chars().collect(), p: 0, depth: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.p < p.c.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}
