//! A minimal YAML reader, sufficient for `docker-compose.yml` and
//! Kubernetes manifests. It supports block mappings, block sequences
//! (including `- key: value` items), plain/quoted scalars, literal and
//! folded block scalars (`|`, `>`), flow sequences `[a, b]` and flow
//! mappings `{a: 1}` — including nested ones, such as the
//! `{requests: {cpu: 2, memory: 8Gi}}` people write by hand, to a depth of
//! `MAX_DEPTH` — `#` comments, and multi-document streams via `parse_all`.
//! Anchors, aliases and tags are not supported and produce an error rather
//! than a wrong tree. Kept in-tree so the toolchain stays dependency-free.

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Scalar(String),
    Seq(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_seq(&self) -> Option<&[Node]> {
        match self {
            Node::Seq(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_map(&self) -> Option<&[(String, Node)]> {
        match self {
            Node::Map(m) => Some(m),
            _ => None,
        }
    }
}

struct Line {
    indent: usize,
    text: String,
    no: usize,
}

/// Remove a `#` comment (one preceded by whitespace or at column 0),
/// respecting single- and double-quoted strings.
fn strip_comment(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut in_d = false;
    let mut in_s = false;
    for i in 0..chars.len() {
        let c = chars[i];
        if in_d {
            out.push(c);
            if c == '"' && !(i > 0 && chars[i - 1] == '\\') {
                in_d = false;
            }
            continue;
        }
        if in_s {
            out.push(c);
            if c == '\'' {
                in_s = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_d = true;
                out.push(c);
            }
            '\'' => {
                in_s = true;
                out.push(c);
            }
            '#' if i == 0 || chars[i - 1] == ' ' || chars[i - 1] == '\t' => break,
            _ => out.push(c),
        }
    }
    out
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => {
                        out.push('\\');
                        out.push(o);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    t.to_string()
}

/// If `text` is `key: rest` (colon outside quotes, followed by a space or
/// end of line), return the unquoted key and the trimmed rest.
fn key_split(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut in_d = false;
    let mut in_s = false;
    for i in 0..chars.len() {
        let c = chars[i];
        if in_d {
            if c == '"' {
                in_d = false;
            }
            continue;
        }
        if in_s {
            if c == '\'' {
                in_s = false;
            }
            continue;
        }
        match c {
            '"' => in_d = true,
            '\'' => in_s = true,
            ':' => {
                let next = chars.get(i + 1).copied();
                if next.is_none() || next == Some(' ') {
                    let key: String = chars[..i].iter().collect();
                    let rest: String = chars[i + 1..].iter().collect();
                    if key.trim().is_empty() {
                        return None;
                    }
                    return Some((unquote(key.trim()), rest.trim().to_string()));
                }
            }
            _ => {}
        }
    }
    None
}

fn is_seq_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// Split a flow collection's interior on its own commas. A comma inside
/// quotes, or inside a collection nested within this one, belongs to that
/// nesting: splitting on it would cut
/// `{requests: {cpu: 2, memory: 8Gi}}` in the middle of the inner mapping
/// and hand back a tree that quietly says something else.
fn split_flow(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_d = false;
    let mut in_s = false;
    let mut depth = 0usize;
    for c in inner.chars() {
        if in_d {
            cur.push(c);
            if c == '"' {
                in_d = false;
            }
            continue;
        }
        if in_s {
            cur.push(c);
            if c == '\'' {
                in_s = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_d = true;
                cur.push(c);
            }
            '\'' => {
                in_s = true;
                cur.push(c);
            }
            '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' => {
                // Saturating, so an unbalanced closer cannot wrap the depth
                // round and start splitting inside a nested collection.
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    parts
}

/// A flow scalar, sequence or mapping on one line. Either collection may
/// hold further collections, so this recurses on both: `depth` is what
/// keeps a nested `{k: {k: ...}}` or `[[[...]]]` from running the stack
/// out, the same bound block nesting gets in `parse_node`.
fn parse_inline(s: &str, depth: usize, no: usize) -> Result<Node, String> {
    if depth > MAX_DEPTH {
        return Err(format!("line {}: nesting too deep", no));
    }
    let t = s.trim();
    if t.is_empty() || t == "~" || t == "null" {
        return Ok(Node::Null);
    }
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        let mut items = Vec::new();
        for p in split_flow(inner) {
            items.push(parse_inline(&p, depth + 1, no)?);
        }
        return Ok(Node::Seq(items));
    }
    if t.starts_with('{') && t.ends_with('}') {
        let inner = &t[1..t.len() - 1];
        let mut m = Vec::new();
        for p in split_flow(inner) {
            match key_split(p.trim()) {
                Some((k, v)) => m.push((k, parse_inline(&v, depth + 1, no)?)),
                None => m.push((unquote(p.trim()), Node::Null)),
            }
        }
        return Ok(Node::Map(m));
    }
    Ok(Node::Scalar(unquote(t)))
}

/// Literal (`|`) or folded (`>`) block scalar: every following line
/// indented past `indent` belongs to it. Comment stripping has already
/// run on these lines, which is acceptable for the configuration data the
/// importers care about.
fn parse_block_scalar(lines: &[Line], idx: &mut usize, indent: usize, folded: bool) -> Node {
    let mut parts = Vec::new();
    while *idx < lines.len() && lines[*idx].indent > indent {
        parts.push(lines[*idx].text.clone());
        *idx += 1;
    }
    Node::Scalar(parts.join(if folded { " " } else { "\n" }))
}

/// Nesting deeper than this is rejected rather than risked as a stack
/// overflow on adversarial input. It bounds block nesting (as an indent
/// column, in `parse_node`) and flow nesting (as a depth, in
/// `parse_inline`).
const MAX_DEPTH: usize = 128;

fn parse_node(lines: &mut Vec<Line>, idx: &mut usize, indent: usize) -> Result<Node, String> {
    if *idx >= lines.len() {
        return Ok(Node::Null);
    }
    if indent > MAX_DEPTH * 2 {
        return Err(format!("line {}: nesting too deep", lines[*idx].no));
    }
    if lines[*idx].indent != indent {
        return Err(format!("line {}: unexpected indentation", lines[*idx].no));
    }
    let text = lines[*idx].text.clone();
    let no = lines[*idx].no;
    if is_seq_item(&text) {
        return parse_seq(lines, idx, indent);
    }
    if key_split(&text).is_some() {
        return parse_map(lines, idx, indent);
    }
    *idx += 1;
    parse_inline(&text, 0, no)
}

fn parse_map(lines: &mut Vec<Line>, idx: &mut usize, indent: usize) -> Result<Node, String> {
    let mut map: Vec<(String, Node)> = Vec::new();
    while *idx < lines.len() && lines[*idx].indent == indent {
        let text = lines[*idx].text.clone();
        if is_seq_item(&text) {
            break;
        }
        let no = lines[*idx].no;
        let (key, rest) = match key_split(&text) {
            Some(kv) => kv,
            None => return Err(format!("line {}: expected `key: value`", no)),
        };
        if rest.starts_with('&') || rest.starts_with('*') || rest.starts_with('!') {
            return Err(format!("line {}: anchors, aliases and tags are not supported", no));
        }
        *idx += 1;
        let value = if rest.starts_with('|') || rest.starts_with('>') {
            parse_block_scalar(lines, idx, indent, rest.starts_with('>'))
        } else if rest.is_empty() {
            if *idx < lines.len() && lines[*idx].indent > indent {
                let ci = lines[*idx].indent;
                parse_node(lines, idx, ci)?
            } else if *idx < lines.len()
                && lines[*idx].indent == indent
                && is_seq_item(&lines[*idx].text)
            {
                parse_seq(lines, idx, indent)?
            } else {
                Node::Null
            }
        } else {
            parse_inline(&rest, 0, no)?
        };
        map.push((key, value));
    }
    Ok(Node::Map(map))
}

fn parse_seq(lines: &mut Vec<Line>, idx: &mut usize, indent: usize) -> Result<Node, String> {
    let mut items = Vec::new();
    while *idx < lines.len() && lines[*idx].indent == indent && is_seq_item(&lines[*idx].text) {
        let text = lines[*idx].text.clone();
        let no = lines[*idx].no;
        let after_dash = &text[1..];
        let spaces = after_dash.len() - after_dash.trim_start().len();
        let rest = after_dash.trim_start().to_string();
        *idx += 1;
        let item = if rest.is_empty() {
            if *idx < lines.len() && lines[*idx].indent > indent {
                let ci = lines[*idx].indent;
                parse_node(lines, idx, ci)?
            } else {
                Node::Null
            }
        } else if is_seq_item(&rest) {
            return Err(format!("line {}: nested inline sequences (`- - x`) are not supported", no));
        } else if key_split(&rest).is_some() {
            // `- key: value` begins a mapping whose remaining keys follow on
            // the next lines at the column where `key` started.
            let child_indent = indent + 1 + spaces;
            lines.insert(*idx, Line { indent: child_indent, text: rest, no });
            parse_map(lines, idx, child_indent)?
        } else {
            parse_inline(&rest, 0, no)?
        };
        items.push(item);
    }
    Ok(Node::Seq(items))
}

/// Parse one YAML document.
pub fn parse(src: &str) -> Result<Node, String> {
    let mut lines: Vec<Line> = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let no = i + 1;
        let stripped = strip_comment(raw);
        if stripped.trim().is_empty() {
            continue;
        }
        let t = stripped.trim_end();
        let trimmed = t.trim_start_matches(' ');
        if trimmed == "---" || trimmed == "..." {
            if !lines.is_empty() && trimmed == "---" {
                return Err(format!(
                    "line {}: multi-document YAML; use parse_all for streams",
                    no
                ));
            }
            continue;
        }
        if trimmed.starts_with('\t') {
            return Err(format!("line {}: tabs are not allowed for indentation", no));
        }
        let indent = t.len() - trimmed.len();
        lines.push(Line { indent, text: trimmed.to_string(), no });
    }
    if lines.is_empty() {
        return Ok(Node::Null);
    }
    let mut idx = 0usize;
    let first = lines[0].indent;
    let node = parse_node(&mut lines, &mut idx, first)?;
    if idx < lines.len() {
        return Err(format!(
            "line {}: unexpected content (check indentation)",
            lines[idx].no
        ));
    }
    Ok(node)
}

/// Parse a multi-document stream (`---` separators). Empty documents are
/// dropped. Line numbers in errors stay absolute.
pub fn parse_all(src: &str) -> Result<Vec<Node>, String> {
    let mut docs = Vec::new();
    let mut chunk = String::new();
    let mut consumed = 0usize;
    for line in src.lines() {
        consumed += 1;
        if line.trim() == "---" {
            if !chunk.trim().is_empty() {
                let n = parse(&chunk)?;
                if n != Node::Null {
                    docs.push(n);
                }
            }
            chunk = "\n".repeat(consumed);
            continue;
        }
        chunk.push_str(line);
        chunk.push('\n');
    }
    if !chunk.trim().is_empty() {
        let n = parse(&chunk)?;
        if n != Node::Null {
            docs.push(n);
        }
    }
    Ok(docs)
}
