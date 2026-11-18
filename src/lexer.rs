//! Hand-written lexer for BWK. No dependencies. Tracks line/column on
//! every token so diagnostics point at the real source position.

use crate::diag::Diag;

/// The largest number literal accepted: 2^53, the last integer every
/// f64 (and every JSON reader that uses doubles) represents exactly.
pub const MAX_NUMBER: f64 = 9_007_199_254_740_992.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(f64),
    /// A double-quoted string literal, escapes already resolved.
    Str(String),
    Dollar,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Colon,
    Comma,
    Eq,
    Le, // <=
    Ge, // >=
    Slash,
    Dot,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

/// Identifiers start with a letter or underscore and may contain letters,
/// digits, underscores, and internal hyphens (so `backend-api` is one
/// token). A hyphen is only consumed when followed by an alphanumeric.
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut out: Vec<Token> = Vec::new();

    let n = chars.len();
    while i < n {
        let c = chars[i];

        // Whitespace.
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            col += 1;
            continue;
        }

        // Line comments: // ... to end of line.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        // String literals: "..." with \" \\ \n \t escapes. No newlines.
        if c == '"' {
            i += 1;
            col += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < n {
                let d = chars[i];
                if d == '"' {
                    i += 1;
                    col += 1;
                    closed = true;
                    break;
                }
                if d == '\n' {
                    break;
                }
                if d == '\\' && i + 1 < n {
                    let e = chars[i + 1];
                    let resolved = match e {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        't' => '\t',
                        _ => {
                            return Err(Diag::error(
                                line,
                                col,
                                format!("unknown escape `\\{}` in string", e),
                            ))
                        }
                    };
                    s.push(resolved);
                    i += 2;
                    col += 2;
                    continue;
                }
                s.push(d);
                i += 1;
                col += 1;
            }
            if !closed {
                return Err(Diag::error(start_line, start_col, "unterminated string literal"));
            }
            out.push(Token { tok: Tok::Str(s), line: start_line, col: start_col });
            continue;
        }

        // Two-character operators.
        if c == '<' && i + 1 < n && chars[i + 1] == '=' {
            out.push(Token { tok: Tok::Le, line: start_line, col: start_col });
            i += 2;
            col += 2;
            continue;
        }
        if c == '>' && i + 1 < n && chars[i + 1] == '=' {
            out.push(Token { tok: Tok::Ge, line: start_line, col: start_col });
            i += 2;
            col += 2;
            continue;
        }

        // Single-character punctuation.
        let single = match c {
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBrack),
            ']' => Some(Tok::RBrack),
            ':' => Some(Tok::Colon),
            ',' => Some(Tok::Comma),
            '=' => Some(Tok::Eq),
            '/' => Some(Tok::Slash),
            '$' => Some(Tok::Dollar),
            _ => None,
        };
        if let Some(t) = single {
            out.push(Token { tok: t, line: start_line, col: start_col });
            i += 1;
            col += 1;
            continue;
        }

        // A dot is punctuation unless it begins a number (we require a
        // leading digit for numbers, so a bare dot is always punctuation).
        if c == '.' {
            out.push(Token { tok: Tok::Dot, line: start_line, col: start_col });
            i += 1;
            col += 1;
            continue;
        }

        // Numbers: digits with an optional single decimal point.
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                // BWK numbers have at most one decimal point.
                if chars[i] == '.' && s.contains('.') {
                    break;
                }
                // A trailing '.' that is not followed by a digit is
                // punctuation, not part of the number.
                if chars[i] == '.' && (i + 1 >= n || !chars[i + 1].is_ascii_digit()) {
                    break;
                }
                s.push(chars[i]);
                i += 1;
                col += 1;
            }
            let val: f64 = s.parse().map_err(|_| {
                Diag::error(start_line, start_col, format!("invalid number `{}`", s))
            })?;
            // Above 2^53 an f64 no longer holds every integer, and JSON
            // readers that use doubles would see a different number than
            // the one written. No real requirement comes close.
            if val > MAX_NUMBER {
                return Err(Diag::error(
                    start_line,
                    start_col,
                    format!("number `{}` is too large (the largest is {})", s, MAX_NUMBER as u64),
                ));
            }
            out.push(Token { tok: Tok::Num(val), line: start_line, col: start_col });
            continue;
        }

        // Identifiers and keywords (keywords are matched later, by the parser).
        if is_ident_start(c) {
            let mut s = String::new();
            s.push(c);
            i += 1;
            col += 1;
            while i < n {
                let d = chars[i];
                if is_ident_continue(d) {
                    s.push(d);
                    i += 1;
                    col += 1;
                } else if d == '-' && i + 1 < n && chars[i + 1].is_ascii_alphanumeric() {
                    // Internal hyphen, e.g. backend-api.
                    s.push('-');
                    i += 1;
                    col += 1;
                } else {
                    break;
                }
            }
            out.push(Token { tok: Tok::Ident(s), line: start_line, col: start_col });
            continue;
        }

        return Err(Diag::error(start_line, start_col, format!("unexpected character `{}`", c)));
    }

    out.push(Token { tok: Tok::Eof, line, col });
    Ok(out)
}
