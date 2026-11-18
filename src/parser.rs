//! Recursive-descent parser for BWK.
//!
//! Unit checking happens here: value readers such as `read_duration_ms`
//! and `read_data_gb` accept only a value of the right unit type and emit
//! a diagnostic otherwise. That is the "units are types" guarantee —
//! `slo.p95 <= $5` and `mem >= 200 ms` are rejected at parse time, not
//! silently coerced.
//!
//! Parsing is resilient: on an unexpected token the parser records a
//! diagnostic and makes progress rather than aborting, so a single typo
//! does not hide the rest of the file.

use crate::ast::*;
use crate::diag::Diag;
use crate::lexer::{Tok, Token};

pub struct ParseResult {
    pub file: File,
    pub diags: Vec<Diag>,
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
}

/// Duration units, in milliseconds.
fn duration_unit_ms(u: &str) -> Option<f64> {
    match u {
        "ms" => Some(1.0),
        "s" => Some(1_000.0),
        "m" => Some(60_000.0),
        "h" => Some(3_600_000.0),
        "d" => Some(86_400_000.0),
        _ => None,
    }
}

/// Data-size units, in GB (binary: 1 GB = 2^30 bytes).
fn data_unit_gb(u: &str) -> Option<f64> {
    match u {
        "MB" => Some(1.0 / 1024.0),
        "GB" => Some(1.0),
        "TB" => Some(1024.0),
        _ => None,
    }
}

fn is_unit(u: &str) -> bool {
    duration_unit_ms(u).is_some() || data_unit_gb(u).is_some()
}

/// A security/trust level like `S3` or `T2` → its numeric rank.
fn level(prefix: char, s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() == 2 && b[0] == prefix as u8 && b[1].is_ascii_digit() {
        let n = b[1] - b'0';
        if n <= 3 {
            return Some(n);
        }
    }
    None
}

const TOP_LEVEL_KEYWORDS: [&str; 4] = ["workload", "offer", "policy", "import"];

const UNIT_RESOURCES_HINT: &str =
    "workload resources are minimums; write `vcpu >= 2`, `mem >= 4 GB`";
const OFFER_RESOURCES_HINT: &str =
    "an offer states what it provides; write `vcpu = 16`, `mem = 32 GB`";

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Parser { toks, pos: 0, diags: Vec::new() }
    }

    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek().tok, Tok::Eof)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err(&mut self, msg: impl Into<String>) {
        let t = self.peek();
        self.diags.push(Diag::error(t.line, t.col, msg));
    }

    fn warn_at(&mut self, line: usize, col: usize, msg: impl Into<String>) {
        self.diags.push(Diag::warning(line, col, msg));
    }

    fn expect(&mut self, want: &Tok, ctx: &str) -> bool {
        if &self.peek().tok == want {
            self.bump();
            true
        } else {
            let found = describe(&self.peek().tok);
            self.err(format!("expected {} {}, found {}", describe(want), ctx, found));
            false
        }
    }

    /// Expect a specific keyword identifier.
    fn expect_kw(&mut self, kw: &str, ctx: &str) -> bool {
        if let Tok::Ident(w) = &self.peek().tok {
            if w == kw {
                self.bump();
                return true;
            }
        }
        let found = describe(&self.peek().tok);
        self.err(format!("expected `{}` {}, found {}", kw, ctx, found));
        false
    }

    /// Read an identifier, or record an error and return a placeholder.
    fn read_ident(&mut self, ctx: &str) -> String {
        if let Tok::Ident(s) = &self.peek().tok {
            let s = s.clone();
            self.bump();
            s
        } else {
            let found = describe(&self.peek().tok);
            self.err(format!("expected {}, found {}", ctx, found));
            String::from("<error>")
        }
    }

    /// `= <ident>`
    fn read_eq_ident(&mut self, ctx: &str) -> Option<String> {
        if !self.expect(&Tok::Eq, "after field name") {
            return None;
        }
        Some(self.read_ident(ctx))
    }

    fn read_string(&mut self, ctx: &str) -> Option<String> {
        if let Tok::Str(s) = &self.peek().tok {
            let s = s.clone();
            self.bump();
            return Some(s);
        }
        let found = describe(&self.peek().tok);
        self.err(format!("expected {} (a quoted string), found {}", ctx, found));
        None
    }

    /// A number literal (no unit). Returns None and diagnoses if the next
    /// token is not a bare number.
    fn read_number(&mut self, ctx: &str) -> Option<f64> {
        if let Tok::Num(v) = self.peek().tok {
            self.bump();
            Some(v)
        } else {
            let found = describe(&self.peek().tok);
            self.err(format!("expected {}, found {}", ctx, found));
            None
        }
    }

    /// A plain count (vcpu, gpu, requests): a number that takes no unit.
    fn read_count(&mut self, what: &str) -> Option<f64> {
        match &self.peek().tok {
            Tok::Dollar => {
                self.err(format!("`{}` is a count, found a money amount", what));
                self.bump();
                if matches!(self.peek().tok, Tok::Num(_)) {
                    self.bump();
                }
                None
            }
            Tok::Num(v) => {
                let v = *v;
                self.bump();
                if let Tok::Ident(u) = &self.peek().tok {
                    if is_unit(u) {
                        let u = u.clone();
                        self.err(format!("`{}` is a count and takes no unit (found `{}`)", what, u));
                        self.bump();
                        return None;
                    }
                }
                Some(v)
            }
            _ => {
                let found = describe(&self.peek().tok);
                self.err(format!("expected a number for `{}`, found {}", what, found));
                None
            }
        }
    }

    /// A duration value: `<num> <unit>`. Rejects money, data sizes and bare
    /// numbers, which is the unit-as-type check in action.
    fn read_duration_ms(&mut self) -> Option<f64> {
        match &self.peek().tok {
            Tok::Dollar => {
                self.err("expected a duration (e.g. `200 ms`), found a money amount");
                self.bump();
                if matches!(self.peek().tok, Tok::Num(_)) {
                    self.bump();
                }
                None
            }
            Tok::Num(v) => {
                let v = *v;
                self.bump();
                if let Tok::Ident(u) = &self.peek().tok {
                    if let Some(scale) = duration_unit_ms(u) {
                        self.bump();
                        return Some(v * scale);
                    }
                    if data_unit_gb(u).is_some() {
                        let u = u.clone();
                        self.err(format!("`{}` is a data-size unit; durations use ms, s, m, h or d", u));
                        self.bump();
                        return None;
                    }
                }
                self.err("a duration needs a unit (e.g. `200 ms`)");
                None
            }
            _ => {
                let found = describe(&self.peek().tok);
                self.err(format!("expected a duration, found {}", found));
                None
            }
        }
    }

    /// A data size: `<num> <MB|GB|TB>`, normalized to GB.
    fn read_data_gb(&mut self) -> Option<f64> {
        match &self.peek().tok {
            Tok::Dollar => {
                self.err("expected a data size (e.g. `4 GB`), found a money amount");
                self.bump();
                if matches!(self.peek().tok, Tok::Num(_)) {
                    self.bump();
                }
                None
            }
            Tok::Num(v) => {
                let v = *v;
                self.bump();
                if let Tok::Ident(u) = &self.peek().tok {
                    if let Some(scale) = data_unit_gb(u) {
                        self.bump();
                        return Some(v * scale);
                    }
                    if duration_unit_ms(u).is_some() {
                        let u = u.clone();
                        self.err(format!("`{}` is a duration unit; data sizes use MB, GB or TB", u));
                        self.bump();
                        return None;
                    }
                }
                self.err("a data size needs a unit (e.g. `4 GB`)");
                None
            }
            _ => {
                let found = describe(&self.peek().tok);
                self.err(format!("expected a data size, found {}", found));
                None
            }
        }
    }

    /// A money value: `$ <num>`.
    fn read_money(&mut self) -> Option<f64> {
        if !self.expect(&Tok::Dollar, "to begin a money amount") {
            return None;
        }
        self.read_number("a money amount after `$`")
    }

    /// `$ <num> / <period>`
    fn read_budget(&mut self) -> Option<Budget> {
        let amount = self.read_money()?;
        if !self.expect(&Tok::Slash, "between amount and period") {
            return None;
        }
        let period = self.read_ident("a period (e.g. month)");
        Some(Budget { amount, period })
    }

    /// `value_of latency = $M per <duration> per <N> requests`, normalized
    /// to dollars per millisecond per request.
    fn read_value_of(&mut self) -> Option<f64> {
        let what = self.read_ident("`latency` after `value_of`");
        if what != "latency" {
            self.err(format!("only `value_of latency` is defined (found `{}`)", what));
        }
        if !self.expect(&Tok::Eq, "after `value_of latency`") {
            return None;
        }
        let money = self.read_money()?;
        if !self.expect_kw("per", "after the amount") {
            return None;
        }
        let dur = self.read_duration_ms()?;
        if !self.expect_kw("per", "after the duration") {
            return None;
        }
        let n = self.read_count("requests")?;
        if let Tok::Ident(w) = &self.peek().tok {
            if w == "requests" || w == "request" {
                self.bump();
            } else {
                self.err("expected `requests` after the request count");
            }
        } else {
            self.err("expected `requests` after the request count");
        }
        if dur <= 0.0 || n <= 0.0 {
            self.err("value_of needs a positive duration and request count");
            return None;
        }
        Some(money / dur / n)
    }

    /// A sensitivity level `S0..S3`.
    fn read_sensitivity(&mut self) -> Option<u8> {
        if let Tok::Ident(s) = &self.peek().tok {
            if let Some(n) = level('S', s) {
                self.bump();
                return Some(n);
            }
            let s = s.clone();
            self.err(format!("`{}` is not a sensitivity level (use S0, S1, S2, or S3)", s));
            self.bump();
            return None;
        }
        self.err("expected a sensitivity level (S0..S3)");
        None
    }

    /// A trust level `T0..T3`.
    fn read_trust(&mut self) -> Option<u8> {
        if let Tok::Ident(s) = &self.peek().tok {
            if let Some(n) = level('T', s) {
                self.bump();
                return Some(n);
            }
            let s = s.clone();
            self.err(format!("`{}` is not a trust level (use T0, T1, T2, or T3)", s));
            self.bump();
            return None;
        }
        self.err("expected a trust level (T0..T3)");
        None
    }

    fn read_bool(&mut self) -> Option<bool> {
        if let Tok::Ident(s) = &self.peek().tok {
            match s.as_str() {
                "true" => {
                    self.bump();
                    return Some(true);
                }
                "false" => {
                    self.bump();
                    return Some(false);
                }
                _ => {}
            }
        }
        self.err("expected `true` or `false`");
        None
    }

    /// `[ ident (, ident)* ]`
    fn read_ident_list(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.expect(&Tok::LBrack, "to begin a list") {
            return out;
        }
        if matches!(self.peek().tok, Tok::RBrack) {
            self.bump();
            return out;
        }
        loop {
            out.push(self.read_ident("a list element"));
            match self.peek().tok {
                Tok::Comma => {
                    self.bump();
                    if matches!(self.peek().tok, Tok::RBrack) {
                        self.bump();
                        break;
                    }
                }
                Tok::RBrack => {
                    self.bump();
                    break;
                }
                _ => {
                    self.err("expected `,` or `]` in list");
                    break;
                }
            }
        }
        out
    }

    /// The operator in a resources block: `>=` on units, `=` on offers.
    /// A mix-up is diagnosed with a hint, and the value is still read so
    /// the rest of the block parses.
    fn expect_resource_op(&mut self, op: &Tok, hint: &str) -> bool {
        if &self.peek().tok == op {
            self.bump();
            return true;
        }
        if matches!(self.peek().tok, Tok::Eq | Tok::Ge | Tok::Le) {
            self.err(hint.to_string());
            self.bump();
            return true;
        }
        let found = describe(&self.peek().tok);
        self.err(format!("expected {} in resources, found {}", describe(op), found));
        false
    }

    /// `resources { vcpu >= 2  mem >= 4 GB  disk >= 20 GB  gpu >= 1 }`
    fn parse_resources(&mut self, op: Tok, hint: &str) -> Resources {
        let mut r = Resources::default();
        if !self.expect(&Tok::LBrace, "to open the resources block") {
            return r;
        }
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("resources") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "vcpu" | "gpu" => {
                    self.bump();
                    if self.expect_resource_op(&op, hint) {
                        let v = self.read_count(&kw);
                        if kw == "vcpu" {
                            r.vcpu = v;
                        } else {
                            r.gpu = v;
                        }
                    }
                }
                "mem" | "disk" => {
                    self.bump();
                    if self.expect_resource_op(&op, hint) {
                        let v = self.read_data_gb();
                        if kw == "mem" {
                            r.mem_gb = v;
                        } else {
                            r.disk_gb = v;
                        }
                    }
                }
                other => self.unknown_field(other, "resources"),
            }
        }
        self.expect(&Tok::RBrace, "to close the resources block");
        r
    }

    // --- top level -------------------------------------------------------

    fn parse_file(&mut self) -> File {
        let mut file = File::default();
        while !self.at_eof() {
            match &self.peek().tok {
                Tok::Ident(kw) if kw == "workload" => {
                    if let Some(w) = self.parse_workload() {
                        file.workloads.push(w);
                    }
                }
                Tok::Ident(kw) if kw == "offer" => {
                    if let Some(o) = self.parse_offer() {
                        file.offers.push(o);
                    }
                }
                Tok::Ident(kw) if kw == "policy" => {
                    if let Some(p) = self.parse_policy() {
                        file.policies.push(p);
                    }
                }
                Tok::Ident(kw) if kw == "import" => {
                    let open = self.bump();
                    if let Tok::Str(path) = &self.peek().tok {
                        let path = path.clone();
                        self.bump();
                        file.imports.push(Import { path, line: open.line, col: open.col });
                    } else {
                        self.err("expected a quoted path after `import`");
                        self.recover_top_level();
                    }
                }
                Tok::Ident(other) => {
                    let other = other.clone();
                    self.err(format!(
                        "expected `workload`, `offer`, `policy` or `import`, found `{}`",
                        other
                    ));
                    self.recover_top_level();
                }
                _ => {
                    let found = describe(&self.peek().tok);
                    self.err(format!(
                        "expected `workload`, `offer`, `policy` or `import`, found {}",
                        found
                    ));
                    self.recover_top_level();
                }
            }
        }
        file
    }

    /// Skip to the next top-level keyword so one stray token does not
    /// derail the whole parse.
    fn recover_top_level(&mut self) {
        if !self.at_eof() {
            self.bump();
        }
        while !self.at_eof() {
            if let Tok::Ident(kw) = &self.peek().tok {
                if TOP_LEVEL_KEYWORDS.contains(&kw.as_str()) {
                    return;
                }
            }
            self.bump();
        }
    }

    /// Recover inside a block: advance to the next token that could begin
    /// a new field, or to the closing brace.
    fn skip_to_block_item(&mut self) {
        if !self.at_eof() {
            self.bump();
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            match self.peek().tok {
                Tok::LBrace => {
                    depth += 1;
                    self.bump();
                }
                Tok::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.bump();
                }
                Tok::Ident(_) if depth == 0 => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn unknown_field(&mut self, field: &str, block: &str) {
        let (l, c) = (self.peek().line, self.peek().col);
        self.diags.push(Diag::error(
            l,
            c,
            format!("unknown field or block `{}` in `{}`", field, block),
        ));
        self.skip_to_block_item();
    }

    /// Read the keyword that starts the next field in a block, or recover.
    fn next_field_keyword(&mut self, block: &str) -> Option<String> {
        match &self.peek().tok {
            Tok::Ident(s) => Some(s.clone()),
            _ => {
                self.err(format!("expected a field inside `{}`", block));
                self.skip_to_block_item();
                None
            }
        }
    }

    fn parse_policy(&mut self) -> Option<Policy> {
        let open = self.bump(); // `policy`
        let name = self.read_ident("a policy name");
        if !self.expect(&Tok::LBrace, "to open the policy body") {
            return None;
        }
        let mut p = Policy {
            name,
            residency: None,
            min_trust: None,
            deny_vendors: Vec::new(),
            deny_countries: Vec::new(),
            deny_jurisdictions: Vec::new(),
            keys: None,
            budget_cap: None,
            line: open.line,
            col: open.col,
        };
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("policy") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "residency" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `residency`") {
                        p.residency = Some(self.read_ident("a residency region"));
                    }
                }
                "min_trust" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `min_trust`") {
                        p.min_trust = self.read_trust();
                    }
                }
                "deny" => {
                    self.bump();
                    let what = self.read_ident("`vendors`, `countries` or `jurisdictions` after `deny`");
                    if self.expect(&Tok::Eq, "after the deny target") {
                        let list = self.read_ident_list();
                        match what.as_str() {
                            "vendors" => p.deny_vendors.extend(list),
                            "countries" => p.deny_countries.extend(list),
                            "jurisdictions" => p.deny_jurisdictions.extend(list),
                            other => {
                                let (l, c) = (open.line, open.col);
                                self.diags.push(Diag::error(
                                    l,
                                    c,
                                    format!(
                                        "`deny {}` is not defined (use `deny vendors`, `deny countries` or `deny jurisdictions`)",
                                        other
                                    ),
                                ));
                            }
                        }
                    }
                }
                "keys" => {
                    self.bump();
                    if let Some(k) = self.read_eq_ident("`customer` or `vendor`") {
                        p.keys = Some(k);
                    }
                }
                "budget" => {
                    self.bump();
                    if matches!(self.peek().tok, Tok::Eq) {
                        self.err("a policy budget is a cap; write `budget <= $N / period`");
                        self.bump();
                        let _ = self.read_budget();
                    } else if self.expect(&Tok::Le, "after `budget` (policies cap, so use `<=`)") {
                        p.budget_cap = self.read_budget();
                    }
                }
                other => self.unknown_field(other, "policy"),
            }
        }
        self.expect(&Tok::RBrace, "to close the policy body");
        Some(p)
    }

    fn parse_workload(&mut self) -> Option<Workload> {
        let open = self.bump(); // `workload`
        let name = self.read_ident("a workload name");
        if !self.expect(&Tok::LBrace, "to open the workload body") {
            return None;
        }
        let mut w = Workload {
            name,
            profile: None,
            budget: None,
            value_latency: None,
            data: Vec::new(),
            services: Vec::new(),
            jobs: Vec::new(),
            line: open.line,
            col: open.col,
        };

        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("workload") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "profile" => {
                    self.bump();
                    if let Some(p) = self.read_eq_ident("a profile name") {
                        w.profile = Some(p);
                    }
                }
                "budget" => {
                    self.bump();
                    if matches!(self.peek().tok, Tok::Le) {
                        self.err("a workload budget is a value; write `budget = $N / period` (`<=` is for policies)");
                        self.bump();
                        let _ = self.read_budget();
                    } else if self.expect(&Tok::Eq, "after `budget`") {
                        w.budget = self.read_budget();
                    }
                }
                "value_of" => {
                    self.bump();
                    w.value_latency = self.read_value_of();
                }
                "data" => {
                    self.bump();
                    if let Some(d) = self.parse_data() {
                        w.data.push(d);
                    }
                }
                "service" => {
                    self.bump();
                    if let Some(s) = self.parse_service() {
                        w.services.push(s);
                    }
                }
                "job" => {
                    self.bump();
                    if let Some(j) = self.parse_job() {
                        w.jobs.push(j);
                    }
                }
                other => self.unknown_field(other, "workload"),
            }
        }
        self.expect(&Tok::RBrace, "to close the workload body");
        Some(w)
    }

    fn parse_data(&mut self) -> Option<DataDecl> {
        let line = self.peek().line;
        let col = self.peek().col;
        let name = self.read_ident("a data block name");
        if !self.expect(&Tok::LBrace, "to open the data block") {
            return None;
        }
        let mut d = DataDecl { name, sensitivity: None, residency: None, keys: None, line, col };
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("data") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "sensitivity" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `sensitivity`") {
                        d.sensitivity = self.read_sensitivity();
                    }
                }
                "residency" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `residency`") {
                        d.residency = Some(self.read_ident("a residency region"));
                    }
                }
                "keys" => {
                    self.bump();
                    if let Some(k) = self.read_eq_ident("`customer` or `vendor`") {
                        d.keys = Some(k);
                    }
                }
                other => self.unknown_field(other, "data"),
            }
        }
        self.expect(&Tok::RBrace, "to close the data block");
        Some(d)
    }

    fn parse_service(&mut self) -> Option<Service> {
        let line = self.peek().line;
        let col = self.peek().col;
        let name = self.read_ident("a service name");
        if !self.expect(&Tok::Colon, "between service name and class") {
            return None;
        }
        let class = self.read_ident("an asset class");
        if !self.expect(&Tok::LBrace, "to open the service body") {
            return None;
        }
        let mut s = Service {
            name,
            class,
            image: None,
            sensitivity: None,
            residency: None,
            slo_p95_ms: None,
            slo_from: Vec::new(),
            min_vendors: None,
            depends_on: Vec::new(),
            resources: Resources::default(),
            value_latency: None,
            cache: Vec::new(),
            requires: Vec::new(),
            keys: None,
            line,
            col,
        };
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("service") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "image" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `image`") {
                        s.image = self.read_string("an image reference");
                    }
                }
                "sensitivity" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `sensitivity`") {
                        s.sensitivity = self.read_sensitivity();
                    }
                }
                "residency" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `residency`") {
                        s.residency = Some(self.read_ident("a residency region"));
                    }
                }
                "slo" => {
                    self.bump();
                    // slo.p95 <= <duration>
                    if self.expect(&Tok::Dot, "after `slo`") {
                        let metric = self.read_ident("an SLO metric (e.g. p95)");
                        if metric != "p95" {
                            self.warn_at(
                                line,
                                col,
                                format!("unknown SLO metric `{}`; only `p95` is defined", metric),
                            );
                        }
                        if self.expect(&Tok::Le, "after the SLO metric") {
                            s.slo_p95_ms = self.read_duration_ms();
                            // Optional: where the users are.
                            if matches!(&self.peek().tok, Tok::Ident(w) if w == "from") {
                                self.bump();
                                if matches!(self.peek().tok, Tok::LBrack) {
                                    s.slo_from = self.read_ident_list();
                                } else {
                                    s.slo_from = vec![self.read_ident("a region after `from`")];
                                }
                            }
                        }
                    }
                }
                "redundancy" => {
                    self.bump();
                    // redundancy { vendors >= <int> }
                    if self.expect(&Tok::LBrace, "to open the redundancy block") {
                        let field = self.read_ident("`vendors`");
                        if field != "vendors" {
                            self.warn_at(
                                line,
                                col,
                                format!("unknown redundancy field `{}`", field),
                            );
                        }
                        if self.expect(&Tok::Ge, "after `vendors`") {
                            if let Some(v) = self.read_number("a vendor count") {
                                s.min_vendors = Some(v as i64);
                            }
                        }
                        self.expect(&Tok::RBrace, "to close the redundancy block");
                    }
                }
                "depends_on" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `depends_on`") {
                        s.depends_on.push(self.read_ident("a data block name"));
                    }
                }
                "resources" => {
                    self.bump();
                    s.resources = self.parse_resources(Tok::Ge, UNIT_RESOURCES_HINT);
                }
                "value_of" => {
                    self.bump();
                    s.value_latency = self.read_value_of();
                }
                "cache" => {
                    // cache "<path>" for <duration>   |   cache "<path>" never
                    let (cline, ccol) = (self.peek().line, self.peek().col);
                    self.bump();
                    if let Some(path) = self.read_string("a path pattern after `cache`") {
                        let mode = match &self.peek().tok {
                            Tok::Ident(w) if w == "never" => 1,
                            Tok::Ident(w) if w == "for" => 2,
                            _ => 0,
                        };
                        match mode {
                            1 => {
                                self.bump();
                                s.cache.push(CacheRule { path, ttl_ms: None, line: cline, col: ccol });
                            }
                            2 => {
                                self.bump();
                                if let Some(ms) = self.read_duration_ms() {
                                    s.cache.push(CacheRule { path, ttl_ms: Some(ms), line: cline, col: ccol });
                                }
                            }
                            _ => self.err("expected `for <duration>` or `never` after the cache path"),
                        }
                    }
                }
                "requires" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `requires`") {
                        s.requires.extend(self.read_ident_list());
                    }
                }
                "keys" => {
                    self.bump();
                    if let Some(k) = self.read_eq_ident("`customer` or `vendor`") {
                        s.keys = Some(k);
                    }
                }
                other => self.unknown_field(other, "service"),
            }
        }
        self.expect(&Tok::RBrace, "to close the service body");
        Some(s)
    }

    fn parse_job(&mut self) -> Option<Job> {
        let line = self.peek().line;
        let col = self.peek().col;
        let name = self.read_ident("a job name");
        if !self.expect(&Tok::Colon, "between job name and class") {
            return None;
        }
        let class = self.read_ident("an asset class");
        if !self.expect(&Tok::LBrace, "to open the job body") {
            return None;
        }
        let mut j = Job {
            name,
            class,
            image: None,
            deadline_ms: None,
            checkpointable: None,
            sensitivity: None,
            residency: None,
            resources: Resources::default(),
            keys: None,
            line,
            col,
        };
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("job") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "image" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `image`") {
                        j.image = self.read_string("an image reference");
                    }
                }
                "deadline" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `deadline`") {
                        j.deadline_ms = self.read_duration_ms();
                    }
                }
                "checkpointable" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `checkpointable`") {
                        j.checkpointable = self.read_bool();
                    }
                }
                "sensitivity" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `sensitivity`") {
                        j.sensitivity = self.read_sensitivity();
                    }
                }
                "residency" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `residency`") {
                        j.residency = Some(self.read_ident("a residency region"));
                    }
                }
                "resources" => {
                    self.bump();
                    j.resources = self.parse_resources(Tok::Ge, UNIT_RESOURCES_HINT);
                }
                "keys" => {
                    self.bump();
                    if let Some(k) = self.read_eq_ident("`customer` or `vendor`") {
                        j.keys = Some(k);
                    }
                }
                other => self.unknown_field(other, "job"),
            }
        }
        self.expect(&Tok::RBrace, "to close the job body");
        Some(j)
    }

    fn parse_offer(&mut self) -> Option<Offer> {
        let open = self.bump(); // `offer`
        let id = self.read_ident("an offer id");
        if !self.expect(&Tok::LBrace, "to open the offer body") {
            return None;
        }
        let mut o = Offer {
            id,
            vendor: None,
            country: None,
            subdivision: None,
            trust: None,
            classes: Vec::new(),
            price: None,
            resources: Resources::default(),
            features: Vec::new(),
            jurisdictions: Vec::new(),
            keys: None,
            latency: Vec::new(),
            line: open.line,
            col: open.col,
        };
        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
            let kw = match self.next_field_keyword("offer") {
                Some(k) => k,
                None => continue,
            };
            match kw.as_str() {
                "vendor" => {
                    self.bump();
                    if let Some(v) = self.read_eq_ident("a vendor name") {
                        o.vendor = Some(v);
                    }
                }
                "country" => {
                    self.bump();
                    if let Some(c) = self.read_eq_ident("a country code") {
                        o.country = Some(c);
                    }
                }
                "trust" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `trust`") {
                        o.trust = self.read_trust();
                    }
                }
                "classes" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `classes`") {
                        o.classes = self.read_ident_list();
                    }
                }
                "price" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `price`") {
                        if let Some(amount) = self.read_money() {
                            if self.expect(&Tok::Slash, "between amount and unit") {
                                let unit = self.read_ident("a price unit (e.g. vcpu_hour)");
                                o.price = Some(Price { amount, unit });
                            }
                        }
                    }
                }
                "resources" => {
                    self.bump();
                    o.resources = self.parse_resources(Tok::Eq, OFFER_RESOURCES_HINT);
                }
                "subdivision" => {
                    // ISO 3166-2 code parts are letters (`ca`, `qc`) or digits (`13`, `18`).
                    self.bump();
                    if self.expect(&Tok::Eq, "after `subdivision`") {
                        match &self.peek().tok {
                            Tok::Ident(s) => {
                                let s = s.clone();
                                self.bump();
                                o.subdivision = Some(s);
                            }
                            Tok::Num(v) if v.fract() == 0.0 && *v >= 0.0 && *v < 1000.0 => {
                                let s = format!("{}", *v as i64);
                                self.bump();
                                o.subdivision = Some(s);
                            }
                            _ => {
                                let found = describe(&self.peek().tok);
                                self.err(format!(
                                    "expected a subdivision code (ISO 3166-2 part, e.g. ca or 13), found {}",
                                    found
                                ));
                            }
                        }
                    }
                }
                "features" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `features`") {
                        o.features = self.read_ident_list();
                    }
                }
                "keys" => {
                    self.bump();
                    if let Some(k) = self.read_eq_ident("`customer` or `vendor`") {
                        o.keys = Some(k);
                    }
                }
                "jurisdictions" => {
                    self.bump();
                    if self.expect(&Tok::Eq, "after `jurisdictions`") {
                        o.jurisdictions = self.read_ident_list();
                    }
                }
                "latency" => {
                    // latency { p95 from <region> = <duration> ... }
                    self.bump();
                    if self.expect(&Tok::LBrace, "to open the latency block") {
                        while !self.at_eof() && !matches!(self.peek().tok, Tok::RBrace) {
                            let metric = self.read_ident("`p95`");
                            if metric != "p95" {
                                self.err(format!("unknown latency metric `{}`; only `p95` is defined", metric));
                                self.skip_to_block_item();
                                continue;
                            }
                            if !matches!(&self.peek().tok, Tok::Ident(w) if w == "from") {
                                self.err("expected `from <region>` after `p95`");
                                self.skip_to_block_item();
                                continue;
                            }
                            self.bump();
                            let region = self.read_ident("a region after `from`");
                            if self.expect(&Tok::Eq, "after the region") {
                                if let Some(ms) = self.read_duration_ms() {
                                    o.latency.push((region, ms));
                                }
                            }
                        }
                        self.expect(&Tok::RBrace, "to close the latency block");
                    }
                }
                other => self.unknown_field(other, "offer"),
            }
        }
        self.expect(&Tok::RBrace, "to close the offer body");
        Some(o)
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{}`", s),
        Tok::Num(_) => "a number".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Dollar => "`$`".into(),
        Tok::LBrace => "`{`".into(),
        Tok::RBrace => "`}`".into(),
        Tok::LBrack => "`[`".into(),
        Tok::RBrack => "`]`".into(),
        Tok::Colon => "`:`".into(),
        Tok::Comma => "`,`".into(),
        Tok::Eq => "`=`".into(),
        Tok::Le => "`<=`".into(),
        Tok::Ge => "`>=`".into(),
        Tok::Slash => "`/`".into(),
        Tok::Dot => "`.`".into(),
        Tok::Eof => "end of file".into(),
    }
}

/// Parse a source string into a `File` plus any diagnostics.
pub fn parse(src: &str) -> ParseResult {
    let toks = match crate::lexer::lex(src) {
        Ok(t) => t,
        Err(d) => {
            return ParseResult { file: File::default(), diags: vec![d] };
        }
    };
    let mut p = Parser::new(toks);
    let file = p.parse_file();
    ParseResult { file, diags: p.diags }
}
