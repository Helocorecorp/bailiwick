//! A minimal TOML reader, sufficient for `fly.toml` and similar config
//! files. Deliberately not a complete TOML implementation: it handles
//! tables, arrays of tables, dotted keys, basic and literal strings,
//! integers, floats, booleans, arrays (including multi-line) and inline
//! tables. Dates/times and multi-line strings are rejected with a clear
//! error rather than mis-parsed. Kept in-tree so the toolchain stays
//! dependency-free.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Value>),
    Table(Table),
}

/// Insertion-ordered table.
pub type Table = Vec<(String, Value)>;

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(t) => t.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// Remove a `#` comment, respecting basic ("...") and literal ('...') strings.
fn strip_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escape = false;
    for c in line.chars() {
        if in_basic {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_basic = false;
            }
            continue;
        }
        if in_literal {
            out.push(c);
            if c == '\'' {
                in_literal = false;
            }
            continue;
        }
        match c {
            '#' => break,
            '"' => {
                in_basic = true;
                out.push(c);
            }
            '\'' => {
                in_literal = true;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Net bracket depth change of a (comment-stripped) line, outside strings.
fn depth_delta(s: &str) -> i32 {
    let mut d = 0i32;
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escape = false;
    for c in s.chars() {
        if in_basic {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_basic = false;
            }
            continue;
        }
        if in_literal {
            if c == '\'' {
                in_literal = false;
            }
            continue;
        }
        match c {
            '"' => in_basic = true,
            '\'' => in_literal = true,
            '[' | '{' => d += 1,
            ']' | '}' => d -= 1,
            _ => {}
        }
    }
    d
}

/// Join physical lines into logical lines so multi-line arrays and inline
/// tables parse as one value. Returns (starting line number, text).
fn logical_lines(src: &str) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, raw) in src.lines().enumerate() {
        let no = i + 1;
        let s = strip_comment(raw);
        if depth == 0 {
            if s.trim().is_empty() {
                continue;
            }
            start = no;
            buf.clear();
            buf.push_str(&s);
        } else {
            buf.push(' ');
            buf.push_str(&s);
        }
        depth += depth_delta(&s);
        if depth < 0 {
            return Err(format!("line {}: unbalanced brackets", no));
        }
        if depth == 0 {
            out.push((start, buf.clone()));
        }
    }
    if depth != 0 {
        return Err(format!("line {}: unterminated array or inline table", start));
    }
    Ok(out)
}

fn split_outside_quotes(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escape = false;
    for c in s.chars() {
        if in_basic {
            cur.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_basic = false;
            }
            continue;
        }
        if in_literal {
            cur.push(c);
            if c == '\'' {
                in_literal = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_basic = true;
                cur.push(c);
            }
            '\'' => {
                in_literal = true;
                cur.push(c);
            }
            c if c == sep => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

fn unquote_key(k: &str) -> String {
    let t = k.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn parse_key_path(s: &str, lineno: usize) -> Result<Vec<String>, String> {
    let parts: Vec<String> = split_outside_quotes(s, '.')
        .into_iter()
        .map(|p| unquote_key(&p))
        .collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("line {}: empty key segment in `{}`", lineno, s));
    }
    Ok(parts)
}

/// Nesting deeper than this is rejected rather than risked as a stack
/// overflow on adversarial input.
const MAX_DEPTH: usize = 128;

struct Cur {
    c: Vec<char>,
    p: usize,
    line: usize,
    depth: usize,
}

impl Cur {
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
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.p += 1;
        }
    }
    fn err(&self, m: &str) -> String {
        format!("line {}: {}", self.line, m)
    }
}

fn parse_value(cur: &mut Cur) -> Result<Value, String> {
    cur.skip_ws();
    if cur.depth > MAX_DEPTH {
        return Err(cur.err("nesting too deep"));
    }
    cur.depth += 1;
    let r = parse_value_inner(cur);
    cur.depth -= 1;
    r
}

fn parse_value_inner(cur: &mut Cur) -> Result<Value, String> {
    match cur.peek() {
        None => Err(cur.err("expected a value")),
        Some('"') => {
            cur.bump();
            let mut s = String::new();
            loop {
                match cur.bump() {
                    None => return Err(cur.err("unterminated string")),
                    Some('"') => break,
                    Some('\\') => match cur.bump() {
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some(o) => return Err(cur.err(&format!("unsupported escape `\\{}`", o))),
                        None => return Err(cur.err("unterminated string")),
                    },
                    Some(c) => s.push(c),
                }
            }
            Ok(Value::Str(s))
        }
        Some('\'') => {
            cur.bump();
            let mut s = String::new();
            loop {
                match cur.bump() {
                    None => return Err(cur.err("unterminated literal string")),
                    Some('\'') => break,
                    Some(c) => s.push(c),
                }
            }
            Ok(Value::Str(s))
        }
        Some('[') => {
            cur.bump();
            let mut items = Vec::new();
            loop {
                cur.skip_ws();
                if cur.peek() == Some(']') {
                    cur.bump();
                    break;
                }
                items.push(parse_value(cur)?);
                cur.skip_ws();
                match cur.peek() {
                    Some(',') => {
                        cur.bump();
                    }
                    Some(']') => {
                        cur.bump();
                        break;
                    }
                    _ => return Err(cur.err("expected `,` or `]` in array")),
                }
            }
            Ok(Value::Array(items))
        }
        Some('{') => {
            cur.bump();
            let mut tbl: Table = Vec::new();
            loop {
                cur.skip_ws();
                if cur.peek() == Some('}') {
                    cur.bump();
                    break;
                }
                let mut key = String::new();
                while let Some(c) = cur.peek() {
                    if c == '=' {
                        break;
                    }
                    key.push(c);
                    cur.bump();
                }
                if cur.peek() != Some('=') {
                    return Err(cur.err("expected `=` in inline table"));
                }
                cur.bump();
                let key = unquote_key(&key);
                if key.is_empty() {
                    return Err(cur.err("empty key in inline table"));
                }
                let v = parse_value(cur)?;
                tbl.push((key, v));
                cur.skip_ws();
                match cur.peek() {
                    Some(',') => {
                        cur.bump();
                    }
                    Some('}') => {
                        cur.bump();
                        break;
                    }
                    _ => return Err(cur.err("expected `,` or `}` in inline table")),
                }
            }
            Ok(Value::Table(tbl))
        }
        Some(c) if c.is_ascii_alphabetic() => {
            let mut w = String::new();
            while let Some(c) = cur.peek() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    w.push(c);
                    cur.bump();
                } else {
                    break;
                }
            }
            match w.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                other => Err(cur.err(&format!("unsupported value `{}`", other))),
            }
        }
        Some(c) if c.is_ascii_digit() || c == '-' || c == '+' => {
            let mut w = String::new();
            while let Some(c) = cur.peek() {
                if c.is_ascii_alphanumeric() || "+-._:".contains(c) {
                    w.push(c);
                    cur.bump();
                } else {
                    break;
                }
            }
            if w.contains(':') || w[1..].contains('-') {
                return Err(cur.err("dates and times are not supported"));
            }
            let clean: String = w.chars().filter(|c| *c != '_').collect();
            if clean.contains('.') || clean.contains('e') || clean.contains('E') {
                clean
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| cur.err(&format!("invalid number `{}`", w)))
            } else {
                clean
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| cur.err(&format!("invalid number `{}`", w)))
            }
        }
        Some(c) => Err(cur.err(&format!("unexpected character `{}`", c))),
    }
}

/// Descend to (creating as needed) the table at `path`. If a segment names
/// an array of tables, descend into its last element — TOML semantics for
/// `[[x]]` followed by `[x.y]`.
fn navigate_mut<'a>(root: &'a mut Table, path: &[String], lineno: usize) -> Result<&'a mut Table, String> {
    let mut cur = root;
    for seg in path {
        let idx = match cur.iter().position(|(k, _)| k == seg) {
            Some(i) => i,
            None => {
                cur.push((seg.clone(), Value::Table(Vec::new())));
                cur.len() - 1
            }
        };
        let entry = &mut cur[idx].1;
        cur = match entry {
            Value::Table(t) => t,
            Value::Array(items) => match items.last_mut() {
                Some(Value::Table(t)) => t,
                _ => return Err(format!("line {}: `{}` is not a table", lineno, seg)),
            },
            _ => return Err(format!("line {}: `{}` is not a table", lineno, seg)),
        };
    }
    Ok(cur)
}

fn push_array_table(root: &mut Table, path: &[String], lineno: usize) -> Result<(), String> {
    let (last, prefix) = path
        .split_last()
        .ok_or_else(|| format!("line {}: empty table-array header", lineno))?;
    let parent = navigate_mut(root, prefix, lineno)?;
    match parent.iter().position(|(k, _)| k == last) {
        None => parent.push((last.clone(), Value::Array(vec![Value::Table(Vec::new())]))),
        Some(i) => match &mut parent[i].1 {
            Value::Array(items) => items.push(Value::Table(Vec::new())),
            _ => return Err(format!("line {}: `{}` is not an array of tables", lineno, last)),
        },
    }
    Ok(())
}

fn insert_dotted(tbl: &mut Table, key_path: &[String], value: Value, lineno: usize) -> Result<(), String> {
    let (last, prefix) = key_path
        .split_last()
        .ok_or_else(|| format!("line {}: empty key", lineno))?;
    let target = navigate_mut(tbl, prefix, lineno)?;
    if target.iter().any(|(k, _)| k == last) {
        return Err(format!("line {}: duplicate key `{}`", lineno, last));
    }
    target.push((last.clone(), value));
    Ok(())
}

/// Parse TOML text into its root table.
pub fn parse(src: &str) -> Result<Value, String> {
    let mut root: Table = Vec::new();
    let mut current: Vec<String> = Vec::new();

    for (lineno, text) in logical_lines(src)? {
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(inner) = t.strip_prefix("[[") {
            let inner = inner
                .strip_suffix("]]")
                .ok_or_else(|| format!("line {}: malformed `[[table]]` header", lineno))?;
            let path = parse_key_path(inner.trim(), lineno)?;
            push_array_table(&mut root, &path, lineno)?;
            current = path;
        } else if let Some(inner) = t.strip_prefix('[') {
            let inner = inner
                .strip_suffix(']')
                .ok_or_else(|| format!("line {}: malformed `[table]` header", lineno))?;
            let path = parse_key_path(inner.trim(), lineno)?;
            navigate_mut(&mut root, &path, lineno)?;
            current = path;
        } else {
            let parts = split_outside_quotes(t, '=');
            if parts.len() < 2 {
                return Err(format!("line {}: expected `key = value`", lineno));
            }
            let key_path = parse_key_path(parts[0].trim(), lineno)?;
            let rest = parts[1..].join("=");
            let mut cur = Cur { c: rest.chars().collect(), p: 0, line: lineno, depth: 0 };
            let value = parse_value(&mut cur)?;
            cur.skip_ws();
            if cur.peek().is_some() {
                return Err(cur.err("unexpected trailing characters after value"));
            }
            let tbl = navigate_mut(&mut root, &current, lineno)?;
            insert_dotted(tbl, &key_path, value, lineno)?;
        }
    }
    Ok(Value::Table(root))
}
