//! A minimal HCL reader, sufficient for Terraform configuration. It
//! recovers the block structure (`resource "type" "name" { ... }`, nested
//! blocks, labels) and attribute values: strings (interpolations kept as
//! raw text), heredocs, numbers, booleans, lists and objects. Expressions
//! that are not literals — references like `aws_db_instance.main.address`,
//! function calls, conditionals — are captured verbatim as `Raw` so that
//! the importer can scan them for cross-resource references without
//! evaluating anything. Kept in-tree so the toolchain stays dependency-free.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    List(Vec<Value>),
    Obj(Vec<(String, Value)>),
    /// A non-literal expression, verbatim.
    Raw(String),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The text of a string or a raw expression.
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Raw(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_obj()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// Every string-ish leaf (Str and Raw), recursively.
    pub fn leaves<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Value::Str(s) | Value::Raw(s) => out.push(s),
            Value::List(l) => {
                for v in l {
                    v.leaves(out);
                }
            }
            Value::Obj(o) => {
                for (_, v) in o {
                    v.leaves(out);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub attrs: Vec<(String, Value)>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: String,
    pub labels: Vec<String>,
    pub body: Body,
    pub line: usize,
}

impl Body {
    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn blocks_of(&self, kind: &str) -> Vec<&Block> {
        self.blocks.iter().filter(|b| b.kind == kind).collect()
    }
    pub fn block(&self, kind: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.kind == kind)
    }
    /// Every string-ish leaf in attributes and nested blocks.
    pub fn leaves<'a>(&'a self, out: &mut Vec<&'a str>) {
        for (_, v) in &self.attrs {
            v.leaves(out);
        }
        for b in &self.blocks {
            b.body.leaves(out);
        }
    }
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

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

impl Cur {
    fn peek(&self) -> Option<char> {
        self.c.get(self.p).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.c.get(self.p + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let r = self.peek();
        if let Some(ch) = r {
            self.p += 1;
            if ch == '\n' {
                self.line += 1;
            }
        }
        r
    }
    fn err(&self, m: &str) -> String {
        format!("line {}: {}", self.line, m)
    }
    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\r')) {
            self.bump();
        }
    }
    fn skip_to_eol(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
        }
    }
    /// Whitespace, newlines and all three comment forms.
    fn skip_trivia(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => {
                    self.bump();
                }
                Some('#') => self.skip_to_eol(),
                Some('/') if self.peek2() == Some('/') => self.skip_to_eol(),
                Some('/') if self.peek2() == Some('*') => {
                    self.bump();
                    self.bump();
                    loop {
                        match self.peek() {
                            None => return Err(self.err("unterminated /* comment")),
                            Some('*') if self.peek2() == Some('/') => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            _ => {
                                self.bump();
                            }
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }
    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }
    /// A quoted string. `${...}` and `%{...}` are kept verbatim.
    fn read_quoted(&mut self) -> Result<String, String> {
        self.bump(); // opening quote
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string")),
                Some('"') => return Ok(s),
                Some('\\') => match self.bump() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some(o) => {
                        s.push('\\');
                        s.push(o);
                    }
                    None => return Err(self.err("unterminated string")),
                },
                Some(c @ ('$' | '%')) if self.peek() == Some('{') => {
                    s.push(c);
                    s.push('{');
                    self.bump();
                    let mut depth = 1i32;
                    loop {
                        match self.bump() {
                            None => return Err(self.err("unterminated interpolation")),
                            Some('{') => {
                                depth += 1;
                                s.push('{');
                            }
                            Some('}') => {
                                depth -= 1;
                                s.push('}');
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some('"') => {
                                // A string literal inside the interpolation.
                                s.push('"');
                                loop {
                                    match self.bump() {
                                        None => return Err(self.err("unterminated string")),
                                        Some('\\') => {
                                            s.push('\\');
                                            if let Some(e) = self.bump() {
                                                s.push(e);
                                            }
                                        }
                                        Some('"') => {
                                            s.push('"');
                                            break;
                                        }
                                        Some(x) => s.push(x),
                                    }
                                }
                            }
                            Some(x) => s.push(x),
                        }
                    }
                }
                Some(c) => s.push(c),
            }
        }
    }
}

fn parse_heredoc(cur: &mut Cur) -> Result<Value, String> {
    cur.bump();
    cur.bump(); // <<
    if cur.peek() == Some('-') {
        cur.bump();
    }
    let marker = cur.read_ident();
    if marker.is_empty() {
        return Err(cur.err("expected a heredoc marker after `<<`"));
    }
    cur.skip_to_eol();
    cur.bump(); // newline
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    loop {
        match cur.bump() {
            None => {
                if line.trim() == marker {
                    break;
                }
                return Err(cur.err(&format!("unterminated heredoc (missing `{}`)", marker)));
            }
            Some('\n') => {
                if line.trim() == marker {
                    break;
                }
                lines.push(std::mem::take(&mut line));
            }
            Some(c) => line.push(c),
        }
    }
    Ok(Value::Str(lines.join("\n")))
}

fn parse_expr(cur: &mut Cur) -> Result<Value, String> {
    if cur.depth > MAX_DEPTH {
        return Err(cur.err("nesting too deep"));
    }
    cur.depth += 1;
    let r = parse_expr_inner(cur);
    cur.depth -= 1;
    r
}

fn parse_expr_inner(cur: &mut Cur) -> Result<Value, String> {
    cur.skip_spaces();
    match cur.peek() {
        None => Err(cur.err("expected a value")),
        Some('"') => Ok(Value::Str(cur.read_quoted()?)),
        Some('<') if cur.peek2() == Some('<') => parse_heredoc(cur),
        Some('[') => {
            cur.bump();
            let mut items = Vec::new();
            loop {
                cur.skip_trivia()?;
                match cur.peek() {
                    None => return Err(cur.err("unterminated list")),
                    Some(']') => {
                        cur.bump();
                        break;
                    }
                    _ => {}
                }
                items.push(parse_expr(cur)?);
                cur.skip_trivia()?;
                match cur.peek() {
                    Some(',') => {
                        cur.bump();
                    }
                    Some(']') => {
                        cur.bump();
                        break;
                    }
                    _ => return Err(cur.err("expected `,` or `]` in list")),
                }
            }
            Ok(Value::List(items))
        }
        Some('{') => {
            cur.bump();
            let mut obj = Vec::new();
            loop {
                cur.skip_trivia()?;
                match cur.peek() {
                    None => return Err(cur.err("unterminated object")),
                    Some('}') => {
                        cur.bump();
                        break;
                    }
                    _ => {}
                }
                let key = if cur.peek() == Some('"') {
                    cur.read_quoted()?
                } else {
                    let k = cur.read_ident();
                    if k.is_empty() {
                        return Err(cur.err("expected a key in object"));
                    }
                    k
                };
                cur.skip_spaces();
                match cur.peek() {
                    Some('=') | Some(':') => {
                        cur.bump();
                    }
                    _ => return Err(cur.err("expected `=` in object")),
                }
                let v = parse_expr(cur)?;
                obj.push((key, v));
                cur.skip_trivia()?;
                if cur.peek() == Some(',') {
                    cur.bump();
                }
            }
            Ok(Value::Obj(obj))
        }
        Some(c)
            if c.is_ascii_digit()
                || (c == '-' && cur.peek2().is_some_and(|d| d.is_ascii_digit())) =>
        {
            let mut s = String::new();
            while let Some(c) = cur.peek() {
                if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '-' || c == '+' {
                    s.push(c);
                    cur.bump();
                } else {
                    break;
                }
            }
            s.parse::<f64>()
                .map(Value::Num)
                .map_err(|_| cur.err(&format!("invalid number `{}`", s)))
        }
        Some(c) if is_ident_start(c) => {
            let word = cur.read_ident();
            match word.as_str() {
                "true" => return Ok(Value::Bool(true)),
                "false" => return Ok(Value::Bool(false)),
                "null" => return Ok(Value::Null),
                _ => {}
            }
            // A reference, function call or other expression: capture it
            // verbatim up to the end of the line or the enclosing delimiter.
            let mut raw = word;
            let mut depth = 0i32;
            loop {
                let c = match cur.peek() {
                    None => break,
                    Some(c) => c,
                };
                if depth == 0 {
                    if c == '\n' || c == ',' || c == ']' || c == '}' || c == ')' {
                        break;
                    }
                    if c == '#' || (c == '/' && matches!(cur.peek2(), Some('/') | Some('*'))) {
                        break;
                    }
                }
                if c == '"' {
                    let s = cur.read_quoted()?;
                    raw.push('"');
                    raw.push_str(&s);
                    raw.push('"');
                    continue;
                }
                if c == '(' || c == '[' || c == '{' {
                    depth += 1;
                } else if c == ')' || c == ']' || c == '}' {
                    depth -= 1;
                }
                raw.push(c);
                cur.bump();
            }
            Ok(Value::Raw(raw.trim().to_string()))
        }
        Some(c) => Err(cur.err(&format!("unexpected character `{}` in expression", c))),
    }
}

fn parse_body(cur: &mut Cur, top: bool) -> Result<Body, String> {
    if cur.depth > MAX_DEPTH {
        return Err(cur.err("nesting too deep"));
    }
    cur.depth += 1;
    let r = parse_body_inner(cur, top);
    cur.depth -= 1;
    r
}

fn parse_body_inner(cur: &mut Cur, top: bool) -> Result<Body, String> {
    let mut body = Body::default();
    loop {
        cur.skip_trivia()?;
        match cur.peek() {
            None => {
                if top {
                    return Ok(body);
                }
                return Err(cur.err("unexpected end of file; expected `}`"));
            }
            Some('}') => {
                if top {
                    return Err(cur.err("unexpected `}`"));
                }
                return Ok(body);
            }
            Some(c) if is_ident_start(c) => {
                let line = cur.line;
                let name = cur.read_ident();
                cur.skip_spaces();
                if cur.peek() == Some('=') {
                    cur.bump();
                    let v = parse_expr(cur)?;
                    body.attrs.push((name, v));
                    continue;
                }
                // A block: labels until `{`.
                let mut labels = Vec::new();
                loop {
                    cur.skip_spaces();
                    match cur.peek() {
                        Some('"') => labels.push(cur.read_quoted()?),
                        Some('{') => break,
                        Some(c) if is_ident_start(c) => labels.push(cur.read_ident()),
                        Some(c) => {
                            return Err(cur.err(&format!(
                                "unexpected `{}` in block header for `{}`",
                                c, name
                            )))
                        }
                        None => return Err(cur.err("unexpected end of file in block header")),
                    }
                }
                cur.bump(); // {
                let inner = parse_body(cur, false)?;
                if cur.peek() != Some('}') {
                    return Err(cur.err("expected `}`"));
                }
                cur.bump();
                body.blocks.push(Block { kind: name, labels, body: inner, line });
            }
            Some(c) => return Err(cur.err(&format!("unexpected character `{}`", c))),
        }
    }
}

/// Parse HCL text into its top-level body.
pub fn parse(src: &str) -> Result<Body, String> {
    let mut cur = Cur { c: src.chars().collect(), p: 0, line: 1, depth: 0 };
    parse_body(&mut cur, true)
}
