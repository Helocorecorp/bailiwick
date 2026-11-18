//! A Language Server Protocol server for BWK, over stdio (`bailiwick lsp`).
//!
//! Features: diagnostics on open/change/save (parse, type and semantic
//! errors, with imports resolved from disk), context-aware completion,
//! hover documentation for every keyword, level, region, class, unit and
//! feature, document symbols, go-to-definition for `depends_on` targets and
//! `import` paths, and whole-document formatting. Full-document sync only.
//!
//! The pure functions (`diagnostics`, `completions`, `hover`, `symbols`,
//! `definition`, `format_edits`) are independent of the transport and are
//! what the tests exercise; `run_stdio` is the thin JSON-RPC loop.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::diag::Diag;
use crate::json::{self, Json};
use crate::lexer::{self, Tok, Token};
use crate::semantics::{self, CLASSES, FEATURES, PROFILES, REGIONS};
use crate::{loader, parser};

// --- URIs and positions ---------------------------------------------------------

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(rest)))
}

pub fn path_to_uri(p: &Path) -> String {
    format!("file://{}", percent_encode(&p.to_string_lossy()))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn line_chars(text: &str, line: usize) -> Vec<char> {
    text.split('\n')
        .nth(line)
        .map(|l| l.trim_end_matches('\r').chars().collect())
        .unwrap_or_default()
}

/// Character index → UTF-16 column, as LSP positions require.
fn utf16_col(chars: &[char], idx: usize) -> usize {
    chars[..idx.min(chars.len())].iter().map(|c| c.len_utf16()).sum()
}

/// UTF-16 column → character index.
fn char_idx(chars: &[char], utf16: usize) -> usize {
    let mut acc = 0usize;
    for (i, c) in chars.iter().enumerate() {
        if acc >= utf16 {
            return i;
        }
        acc += c.len_utf16();
    }
    chars.len()
}

fn pos(line: usize, ch: usize) -> Json {
    Json::obj(vec![("line", Json::Num(line as f64)), ("character", Json::Num(ch as f64))])
}

fn range(l1: usize, c1: usize, l2: usize, c2: usize) -> Json {
    Json::obj(vec![("start", pos(l1, c1)), ("end", pos(l2, c2))])
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

// --- diagnostics ------------------------------------------------------------------

/// Parse and check `text` as the contents of `uri`, resolving imports from
/// disk when the URI names a file.
pub fn analyze(uri: &str, text: &str) -> Vec<Diag> {
    match uri_to_path(uri) {
        Some(p) => loader::load_source(&p, text, Vec::new()).diags,
        None => crate::parse_and_check(text).1,
    }
}

pub fn diag_to_lsp(text: &str, d: &Diag) -> Json {
    let (line, start_char) = if d.line == 0 {
        (0, 0)
    } else {
        (d.line - 1, d.col.saturating_sub(1))
    };
    let chars = line_chars(text, line);
    let start = start_char.min(chars.len());
    let end = if d.line == 0 {
        chars.len()
    } else if start < chars.len() {
        let c = chars[start];
        if is_word_char(c) || c == '$' || c == '"' {
            let mut e = start + 1;
            while e < chars.len() && is_word_char(chars[e]) {
                e += 1;
            }
            e
        } else {
            start + 1
        }
    } else {
        start
    };
    Json::obj(vec![
        ("range", range(line, utf16_col(&chars, start), line, utf16_col(&chars, end))),
        ("severity", Json::Num(if d.is_error() { 1.0 } else { 2.0 })),
        ("source", Json::str("bailiwick")),
        ("message", Json::str(&d.msg)),
    ])
}

pub fn diagnostics(uri: &str, text: &str) -> Vec<Json> {
    analyze(uri, text).iter().map(|d| diag_to_lsp(text, d)).collect()
}

// --- context ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Ctx {
    TopLevel,
    Block(String),
}

const BLOCK_KEYWORDS: &[&str] = &[
    "workload", "offer", "policy", "data", "service", "job", "resources", "redundancy",
];

/// The text before the cursor.
fn prefix(text: &str, line: usize, ch: usize) -> String {
    let mut out = String::new();
    for (i, l) in text.split('\n').enumerate() {
        if i < line {
            out.push_str(l);
            out.push('\n');
        } else if i == line {
            let chars: Vec<char> = l.chars().collect();
            let idx = char_idx(&chars, ch);
            out.extend(chars[..idx.min(chars.len())].iter());
            break;
        }
    }
    out
}

/// Lex, and on failure (an unterminated string being typed, say) retry
/// without the trailing partial token.
fn lex_lenient(src: &str) -> Vec<Token> {
    match lexer::lex(src) {
        Ok(t) => t,
        Err(_) => {
            let cut = src.rfind(|c: char| c == '"' || c.is_whitespace()).unwrap_or(0);
            lexer::lex(&src[..cut]).unwrap_or_default()
        }
    }
}

/// The block the cursor is inside, by walking braces in the prefix.
pub fn context_at(text: &str, line: usize, ch: usize) -> Ctx {
    let toks = lex_lenient(&prefix(text, line, ch));
    let mut stack: Vec<String> = Vec::new();
    let mut stmt: Vec<String> = Vec::new();
    for t in &toks {
        match &t.tok {
            Tok::LBrace => {
                let head = stmt
                    .iter()
                    .rev()
                    .find(|w| BLOCK_KEYWORDS.contains(&w.as_str()))
                    .cloned()
                    .unwrap_or_else(|| "?".to_string());
                stack.push(head);
                stmt.clear();
            }
            Tok::RBrace => {
                stack.pop();
                stmt.clear();
            }
            Tok::Ident(w) => stmt.push(w.clone()),
            _ => {}
        }
    }
    match stack.last() {
        None => Ctx::TopLevel,
        Some(h) => Ctx::Block(h.clone()),
    }
}

// --- completion --------------------------------------------------------------------

fn item(label: &str, kind: u32, detail: &str) -> Json {
    Json::obj(vec![
        ("label", Json::str(label)),
        ("kind", Json::Num(kind as f64)),
        ("detail", Json::str(detail)),
    ])
}

const K_FIELD: u32 = 5;
const K_CLASS: u32 = 7;
const K_VALUE: u32 = 12;
const K_KEYWORD: u32 = 14;
const K_ENUM: u32 = 20;

const SENS_DESC: [&str; 4] = [
    "public data",
    "internal data",
    "customer data under contract",
    "regulated / sensitive data",
];
const TRUST_DESC: [&str; 4] = [
    "agent installed and identity verified; verification tasks only",
    "benchmarks and reliability history verified; public, fault-tolerant, verifiable work",
    "commercial contract or SLA plus a provider-level audit (SOC 2 / ISO 27001)",
    "T2 plus hardware attestation or a sovereign certification; regulated data",
];

pub fn completions(text: &str, line: usize, ch: usize) -> Vec<Json> {
    let ctx = context_at(text, line, ch);
    let chars = line_chars(text, line);
    let idx = char_idx(&chars, ch);
    let line_prefix: String = chars[..idx.min(chars.len())].iter().collect();
    let ltoks = lex_lenient(&line_prefix);
    let first_ident = ltoks.iter().find_map(|t| match &t.tok {
        Tok::Ident(w) => Some(w.clone()),
        _ => None,
    });
    // The field being assigned is the identifier right before the last
    // operator on the line (so `data d { sensitivity = |` → `sensitivity`).
    let field: Option<String> = ltoks
        .iter()
        .rposition(|t| matches!(t.tok, Tok::Eq | Tok::Le | Tok::Ge))
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| match &ltoks[i].tok {
            Tok::Ident(w) => Some(w.clone()),
            _ => None,
        });
    let last_punct = ltoks
        .iter()
        .rev()
        .find(|t| !matches!(t.tok, Tok::Eof | Tok::Ident(_)))
        .map(|t| t.tok.clone());

    let mut items = Vec::new();

    // `service name : |` → classes
    if matches!(last_punct, Some(Tok::Colon))
        && matches!(first_ident.as_deref(), Some("service") | Some("job"))
    {
        for c in CLASSES {
            items.push(item(c, K_CLASS, "asset class"));
        }
        return items;
    }

    // `field = |` → values for that field
    if let Some(field) = field {
        match field.as_str() {
            "residency" => {
                items.push(item("global", K_ENUM, "no residency constraint"));
                for (n, set) in REGIONS {
                    items.push(item(n, K_ENUM, &set.join(" ")));
                }
            }
            "sensitivity" => {
                for (i, d) in SENS_DESC.iter().enumerate() {
                    items.push(item(&format!("S{}", i), K_ENUM, d));
                }
            }
            "trust" | "min_trust" => {
                for (i, d) in TRUST_DESC.iter().enumerate() {
                    items.push(item(&format!("T{}", i), K_ENUM, d));
                }
            }
            "profile" => {
                for p in PROFILES {
                    items.push(item(p, K_ENUM, "profile"));
                }
            }
            "keys" => {
                items.push(item("customer", K_VALUE, "the customer holds the encryption keys"));
                items.push(item("vendor", K_VALUE, "the provider holds the encryption keys"));
            }
            "checkpointable" => {
                items.push(item("true", K_VALUE, "the job can resume from a checkpoint"));
                items.push(item("false", K_VALUE, "the job must run to completion"));
            }
            "requires" | "features" => {
                for f in FEATURES {
                    items.push(item(f, K_ENUM, "edge feature"));
                }
            }
            _ => {}
        }
        return items;
    }

    let fields: &[(&str, &str)] = match &ctx {
        Ctx::TopLevel => &[
            ("workload", "describe a workload: what it needs, not where it runs"),
            ("offer", "describe supply: what a provider offers"),
            ("policy", "an organization policy; it can only narrow"),
            ("import", "pull in another file's policies and offers"),
        ],
        Ctx::Block(b) => match b.as_str() {
            "workload" => &[
                ("profile", "cheapest | balanced | fastest | sovereign | green"),
                ("budget", "budget = $N / month"),
                ("value_of", "value_of latency = $M per <duration> per <N> requests"),
                ("data", "a data store; its sensitivity and residency propagate to dependents"),
                ("service", "service <name> : <class> { ... }"),
                ("job", "job <name> : <class> { ... }"),
            ],
            "data" => &[
                ("sensitivity", "S0..S3; sets the trust floor"),
                ("residency", "a region: data must stay inside it"),
                ("keys", "keys = customer: dependents need customer-held keys"),
            ],
            "service" => &[
                ("image", "image = \"<oci reference>\""),
                ("sensitivity", "S0..S3; sets the trust floor"),
                ("residency", "a region; intersects with dependencies and policies"),
                ("slo.p95", "slo.p95 <= <duration> from <region>"),
                ("redundancy", "redundancy { vendors >= n }"),
                ("depends_on", "depends_on = <data block>"),
                ("resources", "resources { vcpu >= n  mem >= n GB  disk >= n GB  gpu >= n }"),
                ("value_of", "value_of latency = $M per <duration> per <N> requests"),
                ("cache", "cache \"/path/*\" for <duration>  |  cache \"/path/*\" never"),
                ("requires", "requires = [ cache, waf, websockets, ... ]"),
                ("keys", "keys = customer | vendor"),
            ],
            "job" => &[
                ("image", "image = \"<oci reference>\""),
                ("deadline", "deadline = <duration>"),
                ("checkpointable", "checkpointable = true | false"),
                ("sensitivity", "S0..S3; sets the trust floor"),
                ("residency", "a region"),
                ("resources", "resources { vcpu >= n  mem >= n GB  gpu >= n }"),
                ("keys", "keys = customer | vendor"),
            ],
            "offer" => &[
                ("vendor", "vendor = <name>"),
                ("country", "country = <iso 3166-1 alpha-2, lowercase>"),
                ("subdivision", "subdivision = <iso 3166-2 part>, e.g. ca"),
                ("trust", "trust = T0..T3"),
                ("classes", "classes = [ backend-api, ... ]"),
                ("price", "price = $N / <unit>"),
                ("resources", "resources { vcpu = n  mem = n GB  disk = n GB  gpu = n }"),
                ("features", "features = [ cache, waf, ... ]"),
                ("jurisdictions", "jurisdictions = [ us, ... ]: whose law reaches the operator"),
                ("latency", "latency { p95 from eu = 35 ms }: measured"),
                ("keys", "keys = customer | vendor (unstated means vendor)"),
            ],
            "policy" => &[
                ("residency", "intersects with every unit's residency"),
                ("min_trust", "raises every unit's trust floor"),
                ("deny vendors", "deny vendors = [ ... ]"),
                ("deny jurisdictions", "deny jurisdictions = [ ... ]: no plaintext within their reach"),
                ("keys", "keys = customer: every governed unit needs customer-held keys"),
                ("deny countries", "deny countries = [ ... ]"),
                ("budget", "budget <= $N / month (a cap)"),
            ],
            "resources" => &[
                ("vcpu", "a count"),
                ("mem", "a data size: MB | GB | TB"),
                ("disk", "a data size: MB | GB | TB"),
                ("gpu", "a count"),
            ],
            "redundancy" => &[("vendors", "vendors >= n distinct feasible vendors")],
            _ => &[],
        },
    };
    for (f, d) in fields {
        items.push(item(f, if matches!(ctx, Ctx::TopLevel) { K_KEYWORD } else { K_FIELD }, d));
    }
    items
}

// --- hover ------------------------------------------------------------------------------

fn level(prefix: char, s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() == 2 && b[0] == prefix as u8 && b[1].is_ascii_digit() {
        let n = (b[1] - b'0') as usize;
        if n <= 3 {
            return Some(n);
        }
    }
    None
}

pub fn word_at(text: &str, line: usize, ch: usize) -> Option<String> {
    let chars = line_chars(text, line);
    if chars.is_empty() {
        return None;
    }
    let mut idx = char_idx(&chars, ch).min(chars.len());
    if idx == chars.len() || !is_word_char(chars[idx]) {
        if idx > 0 && is_word_char(chars[idx - 1]) {
            idx -= 1;
        } else {
            return None;
        }
    }
    let mut s = idx;
    while s > 0 && is_word_char(chars[s - 1]) {
        s -= 1;
    }
    let mut e = idx;
    while e < chars.len() && is_word_char(chars[e]) {
        e += 1;
    }
    let w: String = chars[s..e].iter().collect();
    if w.is_empty() {
        None
    } else {
        Some(w)
    }
}

/// Documentation for a word, as markdown.
pub fn doc_for(word: &str, ctx: &Ctx) -> Option<String> {
    if let Some(n) = level('S', word) {
        return Some(format!(
            "**{}** — sensitivity level {}: {}. Requires trust **T{}** or higher.",
            word,
            n,
            SENS_DESC[n],
            semantics::min_trust(n as u8)
        ));
    }
    if let Some(n) = level('T', word) {
        return Some(format!("**{}** — trust level {}: {}.", word, n, TRUST_DESC[n]));
    }
    if word == "global" {
        return Some("**global** — no residency constraint; any location is allowed.".to_string());
    }
    if let Some((name, set)) = REGIONS.iter().find(|(n, _)| *n == word) {
        return Some(format!(
            "**{}** — residency region: `{}`. Regions are sets and intersect exactly.",
            name,
            set.join(", ")
        ));
    }
    if CLASSES.contains(&word) {
        let d = match word {
            "frontend-static" => "static assets served from an edge; needs no compute",
            "backend-api" => "request/response services; latency-sensitive",
            "media-assets" => "large objects; egress cost dominates",
            "database" => "stateful stores; placement is residency-dominant",
            "background-jobs" => "deadline-flexible work; eligible for spot and open supply",
            "ml-inference" => "GPU/accelerator-bound serving",
            _ => "",
        };
        return Some(format!("**{}** — asset class: {}.", word, d));
    }
    if FEATURES.contains(&word) {
        return Some(format!(
            "**{}** — edge feature. A service that requires it is feasible only on offers that list it in `features`.",
            word
        ));
    }
    if PROFILES.contains(&word) {
        let d = match word {
            "cheapest" => "weights cost most; 1% of unit price per 10 ms",
            "balanced" => "the default exchange rate; 5% per 10 ms",
            "fastest" => "weights latency most; 20% per 10 ms",
            "sovereign" => "residency required and a trust floor of T2 (T3 for S3 data)",
            "green" => "adds a carbon price",
            _ => "",
        };
        return Some(format!("**{}** — profile (a preference, never a filter): {}.", word, d));
    }
    let unit = match word {
        "ms" => Some("milliseconds"),
        "s" => Some("seconds"),
        "m" => Some("minutes"),
        "h" => Some("hours"),
        "d" => Some("days"),
        "MB" => Some("megabytes (2^20 bytes)"),
        "GB" => Some("gigabytes (2^30 bytes)"),
        "TB" => Some("terabytes (2^40 bytes)"),
        _ => None,
    };
    if let Some(u) = unit {
        return Some(format!("**{}** — unit: {}. Units are types; mixing them is a parse error.", word, u));
    }
    let in_policy = matches!(ctx, Ctx::Block(b) if b == "policy");
    let doc = match word {
        "workload" => "A workload: what it needs, described once. Contains `data`, `service` and `job` blocks.",
        "service" => "`service <name> : <class> { ... }` — a routable, long-running unit.",
        "job" => "`job <name> : <class> { ... }` — deadline-flexible work.",
        "data" => "A data store. Its `sensitivity` and `residency` propagate to every service that `depends_on` it.",
        "offer" => "Supply: what a provider offers — location, trust, classes, price, resources, features.",
        "policy" => "An organization policy. Policies can only narrow: residency intersects, trust floors rise, deny lists union, budgets cap.",
        "import" => "`import \"path.bwk\"` — pulls in that file's policies and offers, resolved relative to this file.",
        "profile" => "`profile = cheapest | balanced | fastest | sovereign | green` — exchange rates for the optimizer, never a filter.",
        "budget" => {
            if in_policy {
                "`budget <= $N / month` — a cap every governed workload's budget must respect."
            } else {
                "`budget = $N / month` — the workload's budget."
            }
        }
        "value_of" | "per" | "requests" => "`value_of latency = $M per <duration> per <N> requests` — what latency is worth; a preference for the optimizer, never a filter.",
        "image" => "`image = \"<oci reference>\"` — the container image; a digest is recommended.",
        "sensitivity" => "`sensitivity = S0..S3`. Sets the trust floor: S0→T1, S1→T2, S2→T2, S3→T3.",
        "residency" => {
            if in_policy {
                "`residency = <region>` — intersects with every governed unit's residency."
            } else {
                "`residency = <region>` — a hard filter: only offers located inside the region are feasible."
            }
        }
        "slo" | "slo.p95" | "p95" => "`slo.p95 <= <duration> from <region>` — the latency target, measured from where your users are. With `from`, a rule: an offer qualifies only with a measured p95 from each region within the target; unmeasured offers are excluded. Without `from`, only a target for choosing among allowed offers.",
        "latency" => "On an offer, `latency { p95 from <region> = <duration> }` — measured p95 latency from users in that region; a service with `slo.p95 <= ... from <region>` needs one within its target. In `value_of latency = ...`, what latency is worth to the optimizer.",
        "from" => "`from <region>` (or `from [ eu, us ]`) — where users are: the SLO must be met from each.",
        "redundancy" | "vendors" => "`redundancy { vendors >= n }` — the feasible set must span n distinct vendors, or the unit fails closed.",
        "depends_on" => "`depends_on = <data>` — the service needs this data; its sensitivity and residency propagate.",
        "resources" => "`resources { ... }` — on a unit, minimums (`>=`); on an offer, what it provides (`=`). Offers must declare and meet every required resource.",
        "vcpu" => "`vcpu` — a count (no unit).",
        "mem" => "`mem` — a data size: `MB`, `GB` or `TB` (binary).",
        "disk" => "`disk` — a data size: `MB`, `GB` or `TB` (binary).",
        "gpu" => "`gpu` — a count (no unit).",
        "cache" => "`cache \"/path/*\" for <duration>` or `cache \"/path/*\" never` — an edge cache rule. Any cache rule implicitly requires the `cache` feature.",
        "never" | "for" => "Part of a cache rule: `cache \"/path/*\" for <duration>` or `cache \"/path/*\" never`.",
        "requires" => "`requires = [ cache, waf, ... ]` — edge features the offer must provide.",
        "deadline" => "`deadline = <duration>` — the job must finish within it.",
        "checkpointable" => "`checkpointable = true | false` — whether the job can resume; checkpointable jobs can use preemptible capacity.",
        "vendor" => "`vendor = <name>` — the provider; `redundancy` counts distinct vendors.",
        "country" => "`country = <iso 3166-1 alpha-2, lowercase>` — where the offer is located.",
        "subdivision" => "`subdivision = <iso 3166-2 part>` (e.g. `ca`) — needed to satisfy a sub-national residency such as `california`.",
        "trust" => "`trust = T0..T3` — the offer's trust level, backed by evidence.",
        "classes" => "`classes = [ ... ]` — the asset classes the offer can serve.",
        "price" => "`price = $N / <unit>` — the offer's price; the optimizer uses invoice-reconciled prices.",
        "features" => "`features = [ cache, waf, ... ]` — edge features the offer provides.",
        "min_trust" => "`min_trust = T0..T3` — raises every governed unit's trust floor.",
        "deny" => "`deny vendors = [ ... ]` / `deny countries = [ ... ]` / `deny jurisdictions = [ ... ]` — removes offers; the plan names the policy that did it.",
        "jurisdictions" => "On an offer, `jurisdictions = [ ... ]`: legal systems, besides the offer's own country, whose law can compel its operator (e.g. the vendor's home country). In a policy, `deny jurisdictions = [ ... ]`: an offer within reach of one of them is excluded unless the customer holds the keys.",
        "keys" => "`keys = customer | vendor` — who holds the encryption keys. On an offer, unstated means `vendor`. On data, a service, a job or a policy, `customer` excludes every offer where the provider holds the keys.",
        _ => return None,
    };
    Some(doc.to_string())
}

pub fn hover(text: &str, line: usize, ch: usize) -> Option<String> {
    let w = word_at(text, line, ch)?;
    let ctx = context_at(text, line, ch);
    doc_for(&w, &ctx)
}

// --- symbols ----------------------------------------------------------------------------

fn symbol(name: &str, kind: u32, detail: &str, text: &str, line: usize, col: usize, children: Vec<Json>) -> Json {
    let l = line.saturating_sub(1);
    let chars = line_chars(text, l);
    let start = utf16_col(&chars, col.saturating_sub(1));
    let end = utf16_col(&chars, chars.len());
    let r = range(l, start, l, end);
    Json::obj(vec![
        ("name", Json::str(name)),
        ("kind", Json::Num(kind as f64)),
        ("detail", Json::str(detail)),
        ("range", r.clone()),
        ("selectionRange", r),
        ("children", Json::Arr(children)),
    ])
}

pub fn symbols(text: &str) -> Vec<Json> {
    let file = parser::parse(text).file;
    let mut out = Vec::new();
    for p in &file.policies {
        out.push(symbol(&p.name, 11, "policy", text, p.line, p.col, vec![]));
    }
    for w in &file.workloads {
        let mut children = Vec::new();
        for d in &w.data {
            children.push(symbol(&d.name, 23, "data", text, d.line, d.col, vec![]));
        }
        for s in &w.services {
            children.push(symbol(&s.name, 5, &format!("service : {}", s.class), text, s.line, s.col, vec![]));
        }
        for j in &w.jobs {
            children.push(symbol(&j.name, 12, &format!("job : {}", j.class), text, j.line, j.col, vec![]));
        }
        out.push(symbol(&w.name, 3, "workload", text, w.line, w.col, children));
    }
    for o in &file.offers {
        out.push(symbol(&o.id, 19, "offer", text, o.line, o.col, vec![]));
    }
    out
}

// --- definition --------------------------------------------------------------------------

pub fn definition(uri: &str, text: &str, line: usize, ch: usize) -> Option<Json> {
    let chars = line_chars(text, line);
    let line_text: String = chars.iter().collect();
    if line_text.trim_start().starts_with("import") {
        let start = line_text.find('"')? + 1;
        let end = line_text[start..].find('"')? + start;
        let rel = &line_text[start..end];
        let base = uri_to_path(uri)?;
        let target = base.parent()?.join(rel);
        return Some(Json::obj(vec![
            ("uri", Json::str(path_to_uri(&target))),
            ("range", range(0, 0, 0, 0)),
        ]));
    }
    if line_text.contains("depends_on") {
        let w = word_at(text, line, ch)?;
        let toks = lex_lenient(text);
        for i in 0..toks.len().saturating_sub(1) {
            if let (Tok::Ident(a), Tok::Ident(b)) = (&toks[i].tok, &toks[i + 1].tok) {
                if a == "data" && *b == w {
                    let t = &toks[i + 1];
                    let l = t.line.saturating_sub(1);
                    let lc = line_chars(text, l);
                    let s = utf16_col(&lc, t.col.saturating_sub(1));
                    let e = utf16_col(&lc, (t.col.saturating_sub(1) + w.chars().count()).min(lc.len()));
                    return Some(Json::obj(vec![
                        ("uri", Json::str(uri)),
                        ("range", range(l, s, l, e)),
                    ]));
                }
            }
        }
    }
    None
}

// --- formatting --------------------------------------------------------------------------

pub fn format_edits(text: &str) -> Vec<Json> {
    let (out, diags) = crate::format_src(text);
    if crate::any_errors(&diags) || out == text {
        return vec![];
    }
    let parts: Vec<&str> = text.split('\n').collect();
    let last = parts.len() - 1;
    let last_chars: Vec<char> = parts[last].chars().collect();
    vec![Json::obj(vec![
        ("range", range(0, 0, last, utf16_col(&last_chars, last_chars.len()))),
        ("newText", Json::str(out)),
    ])]
}

// --- server ---------------------------------------------------------------------------------

#[derive(Default)]
pub struct Server {
    docs: HashMap<String, String>,
}

type RpcError = (i64, String);

fn doc_uri(params: &Json) -> Result<String, RpcError> {
    params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .ok_or((-32602, "missing textDocument.uri".to_string()))
}

fn doc_pos(params: &Json) -> Result<(String, usize, usize), RpcError> {
    let uri = doc_uri(params)?;
    let p = params.get("position").ok_or((-32602, "missing position".to_string()))?;
    let line = p.get("line").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0) as usize;
    let ch = p.get("character").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0) as usize;
    Ok((uri, line, ch))
}

fn publish(uri: &str, text: &str) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("textDocument/publishDiagnostics")),
        (
            "params",
            Json::obj(vec![
                ("uri", Json::str(uri)),
                ("diagnostics", Json::Arr(diagnostics(uri, text))),
            ]),
        ),
    ])
}

impl Server {
    fn text(&self, uri: &str) -> Result<&str, RpcError> {
        self.docs
            .get(uri)
            .map(|s| s.as_str())
            .ok_or((-32602, format!("unknown document {}", uri)))
    }

    pub fn handle_request(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            "initialize" => Ok(Json::obj(vec![
                (
                    "capabilities",
                    Json::obj(vec![
                        ("textDocumentSync", Json::Num(1.0)),
                        (
                            "completionProvider",
                            Json::obj(vec![
                                ("triggerCharacters", Json::strs(&[" ", "=", ":"])),
                                ("resolveProvider", Json::Bool(false)),
                            ]),
                        ),
                        ("hoverProvider", Json::Bool(true)),
                        ("documentSymbolProvider", Json::Bool(true)),
                        ("definitionProvider", Json::Bool(true)),
                        ("documentFormattingProvider", Json::Bool(true)),
                    ]),
                ),
                (
                    "serverInfo",
                    Json::obj(vec![
                        ("name", Json::str("bailiwick")),
                        ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                    ]),
                ),
            ])),
            "shutdown" => Ok(Json::Null),
            "textDocument/completion" => {
                let (uri, line, ch) = doc_pos(params)?;
                let text = self.text(&uri)?;
                Ok(Json::Arr(completions(text, line, ch)))
            }
            "textDocument/hover" => {
                let (uri, line, ch) = doc_pos(params)?;
                let text = self.text(&uri)?;
                Ok(match hover(text, line, ch) {
                    Some(md) => Json::obj(vec![(
                        "contents",
                        Json::obj(vec![("kind", Json::str("markdown")), ("value", Json::str(md))]),
                    )]),
                    None => Json::Null,
                })
            }
            "textDocument/documentSymbol" => {
                let uri = doc_uri(params)?;
                let text = self.text(&uri)?;
                Ok(Json::Arr(symbols(text)))
            }
            "textDocument/definition" => {
                let (uri, line, ch) = doc_pos(params)?;
                let text = self.text(&uri)?;
                Ok(definition(&uri, text, line, ch).unwrap_or(Json::Null))
            }
            "textDocument/formatting" => {
                let uri = doc_uri(params)?;
                let text = self.text(&uri)?;
                Ok(Json::Arr(format_edits(text)))
            }
            _ => Err((-32601, format!("method not found: {}", method))),
        }
    }

    /// Returns notifications to send back (diagnostics).
    pub fn handle_notification(&mut self, method: &str, params: &Json) -> Vec<Json> {
        match method {
            "textDocument/didOpen" => {
                let td = params.get("textDocument");
                let uri = td.and_then(|t| t.get("uri")).and_then(|u| u.as_str());
                let text = td.and_then(|t| t.get("text")).and_then(|u| u.as_str());
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.docs.insert(uri.to_string(), text.to_string());
                    return vec![publish(uri, text)];
                }
                vec![]
            }
            "textDocument/didChange" => {
                let uri = match doc_uri(params) {
                    Ok(u) => u,
                    Err(_) => return vec![],
                };
                let text = params
                    .get("contentChanges")
                    .and_then(|c| c.as_arr())
                    .and_then(|a| a.last())
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());
                if let Some(text) = text {
                    self.docs.insert(uri.clone(), text.clone());
                    return vec![publish(&uri, &text)];
                }
                vec![]
            }
            "textDocument/didSave" => {
                let uri = match doc_uri(params) {
                    Ok(u) => u,
                    Err(_) => return vec![],
                };
                if let Some(text) = params.get("text").and_then(|t| t.as_str()) {
                    self.docs.insert(uri.clone(), text.to_string());
                }
                match self.docs.get(&uri) {
                    Some(text) => vec![publish(&uri, text)],
                    None => vec![],
                }
            }
            "textDocument/didClose" => {
                if let Ok(uri) = doc_uri(params) {
                    self.docs.remove(&uri);
                    return vec![Json::obj(vec![
                        ("jsonrpc", Json::str("2.0")),
                        ("method", Json::str("textDocument/publishDiagnostics")),
                        (
                            "params",
                            Json::obj(vec![("uri", Json::str(uri)), ("diagnostics", Json::Arr(vec![]))]),
                        ),
                    ])];
                }
                vec![]
            }
            _ => vec![],
        }
    }
}

// --- transport ------------------------------------------------------------------------------

fn read_message(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            if content_length.is_some() {
                break;
            }
            continue;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let len = content_length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).to_string()))
}

fn write_message(out: &mut impl Write, body: &Json) -> io::Result<()> {
    let s = json::to_string_compact(body);
    write!(out, "Content-Length: {}\r\n\r\n{}", s.len(), s)?;
    out.flush()
}

/// Serve LSP over stdin/stdout until `exit` or EOF. Returns the exit code.
pub fn run_stdio() -> i32 {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut server = Server::default();
    loop {
        let msg = match read_message(&mut input) {
            Ok(Some(m)) => m,
            Ok(None) => return 0,
            Err(e) => {
                eprintln!("bailiwick lsp: transport error: {}", e);
                return 1;
            }
        };
        let req = match json::parse(&msg) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("bailiwick lsp: bad message: {}", e);
                continue;
            }
        };
        let method = req.get("method").and_then(|m| m.as_str()).map(|s| s.to_string());
        let id = req.get("id").cloned();
        let params = req.get("params").cloned().unwrap_or(Json::Null);
        let result = match (method.as_deref(), id) {
            (Some("exit"), _) => return 0,
            (Some(m), Some(id)) => {
                let resp = match server.handle_request(m, &params) {
                    Ok(result) => Json::obj(vec![
                        ("jsonrpc", Json::str("2.0")),
                        ("id", id),
                        ("result", result),
                    ]),
                    Err((code, message)) => Json::obj(vec![
                        ("jsonrpc", Json::str("2.0")),
                        ("id", id),
                        (
                            "error",
                            Json::obj(vec![("code", Json::Num(code as f64)), ("message", Json::str(message))]),
                        ),
                    ]),
                };
                write_message(&mut out, &resp)
            }
            (Some(m), None) => {
                let mut r = Ok(());
                for n in server.handle_notification(m, &params) {
                    r = write_message(&mut out, &n);
                    if r.is_err() {
                        break;
                    }
                }
                r
            }
            (None, _) => Ok(()), // a response to something we sent; nothing to do
        };
        if let Err(e) = result {
            eprintln!("bailiwick lsp: write error: {}", e);
            return 1;
        }
    }
}
