//! Protobuf (proto3) wire-format codec for the IR, matching
//! `spec/ir/bwk.proto` and `spec/ir/plan.proto` field for field. Written
//! by hand so the toolchain stays dependency-free and so the encoding is
//! *deterministic*: fields in ascending number order, implicit-presence
//! scalars omitted at their default, explicit-presence (`optional`) scalars
//! written whenever set, no unknown fields. Two conforming encoders must
//! produce identical bytes for the same plan — that is what the
//! `*.expected.pb` files in the conformance suite check.

use crate::ast::*;
use crate::feasibility::WorkloadPlan;

const WT_VARINT: u8 = 0;
const WT_FIXED64: u8 = 1;
const WT_LEN: u8 = 2;
const WT_FIXED32: u8 = 5;

// --- writer -------------------------------------------------------------------

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    fn varint(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(b);
                return;
            }
            self.buf.push(b | 0x80);
        }
    }
    fn tag(&mut self, field: u32, wt: u8) {
        self.varint(((field as u64) << 3) | wt as u64);
    }
    pub fn bytes(&mut self, field: u32, b: &[u8]) {
        self.tag(field, WT_LEN);
        self.varint(b.len() as u64);
        self.buf.extend_from_slice(b);
    }
    /// Implicit-presence string: omitted when empty.
    pub fn string(&mut self, field: u32, s: &str) {
        if !s.is_empty() {
            self.bytes(field, s.as_bytes());
        }
    }
    /// Explicit-presence or repeated-element string: always written.
    pub fn string_present(&mut self, field: u32, s: &str) {
        self.bytes(field, s.as_bytes());
    }
    pub fn opt_string(&mut self, field: u32, s: &Option<String>) {
        if let Some(s) = s {
            self.string_present(field, s);
        }
    }
    pub fn double(&mut self, field: u32, v: f64) {
        if v != 0.0 {
            self.double_present(field, v);
        }
    }
    pub fn double_present(&mut self, field: u32, v: f64) {
        self.tag(field, WT_FIXED64);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn opt_double(&mut self, field: u32, v: Option<f64>) {
        if let Some(x) = v {
            self.double_present(field, x);
        }
    }
    pub fn uint(&mut self, field: u32, v: u64) {
        if v != 0 {
            self.uint_present(field, v);
        }
    }
    pub fn uint_present(&mut self, field: u32, v: u64) {
        self.tag(field, WT_VARINT);
        self.varint(v);
    }
    pub fn opt_uint(&mut self, field: u32, v: Option<u64>) {
        if let Some(x) = v {
            self.uint_present(field, x);
        }
    }
    pub fn opt_int64(&mut self, field: u32, v: Option<i64>) {
        if let Some(x) = v {
            self.uint_present(field, x as u64);
        }
    }
    pub fn boolean(&mut self, field: u32, b: bool) {
        if b {
            self.uint_present(field, 1);
        }
    }
    pub fn opt_bool(&mut self, field: u32, b: Option<bool>) {
        if let Some(x) = b {
            self.uint_present(field, x as u64);
        }
    }
    pub fn message(&mut self, field: u32, inner: &Writer) {
        self.bytes(field, &inner.buf);
    }
}

// --- encoders: bwk.proto ----------------------------------------------------------

fn enc_budget(b: &Budget) -> Writer {
    let mut w = Writer::new();
    w.double(1, b.amount);
    w.string(2, &b.period);
    w
}

fn enc_resources(r: &Resources) -> Writer {
    let mut w = Writer::new();
    w.opt_double(1, r.vcpu);
    w.opt_double(2, r.mem_gb);
    w.opt_double(3, r.disk_gb);
    w.opt_double(4, r.gpu);
    w
}

fn enc_policy(p: &Policy) -> Writer {
    let mut w = Writer::new();
    w.string(1, &p.name);
    w.opt_string(2, &p.residency);
    w.opt_uint(3, p.min_trust.map(|x| x as u64));
    for v in &p.deny_vendors {
        w.string_present(4, v);
    }
    for c in &p.deny_countries {
        w.string_present(5, c);
    }
    if let Some(b) = &p.budget_cap {
        w.message(6, &enc_budget(b));
    }
    for j in &p.deny_jurisdictions {
        w.string_present(7, j);
    }
    w.opt_string(8, &p.keys);
    w
}

fn enc_data(d: &DataDecl) -> Writer {
    let mut w = Writer::new();
    w.string(1, &d.name);
    w.opt_uint(2, d.sensitivity.map(|x| x as u64));
    w.opt_string(3, &d.residency);
    w.opt_string(4, &d.keys);
    w
}

fn enc_cache(c: &CacheRule) -> Writer {
    let mut w = Writer::new();
    w.string(1, &c.path);
    w.opt_double(2, c.ttl_ms);
    w.boolean(3, c.ttl_ms.is_none());
    w
}

fn enc_service(s: &Service) -> Writer {
    let mut w = Writer::new();
    w.string(1, &s.name);
    w.string(2, &s.class);
    w.opt_string(3, &s.image);
    w.opt_uint(4, s.sensitivity.map(|x| x as u64));
    w.opt_string(5, &s.residency);
    w.opt_double(6, s.slo_p95_ms);
    w.opt_int64(7, s.min_vendors);
    for d in &s.depends_on {
        w.string_present(8, d);
    }
    if !s.resources.is_empty() {
        w.message(9, &enc_resources(&s.resources));
    }
    w.opt_double(10, s.value_latency);
    for c in &s.cache {
        w.message(11, &enc_cache(c));
    }
    for f in &s.requires {
        w.string_present(12, f);
    }
    w.opt_string(13, &s.keys);
    for r in &s.slo_from {
        w.string_present(14, r);
    }
    w
}

fn enc_job(j: &Job) -> Writer {
    let mut w = Writer::new();
    w.string(1, &j.name);
    w.string(2, &j.class);
    w.opt_string(3, &j.image);
    w.opt_double(4, j.deadline_ms);
    w.opt_bool(5, j.checkpointable);
    w.opt_uint(6, j.sensitivity.map(|x| x as u64));
    w.opt_string(7, &j.residency);
    if !j.resources.is_empty() {
        w.message(8, &enc_resources(&j.resources));
    }
    w.opt_string(9, &j.keys);
    w
}

fn enc_workload(wl: &Workload) -> Writer {
    let mut w = Writer::new();
    w.string(1, &wl.name);
    w.opt_string(2, &wl.profile);
    if let Some(b) = &wl.budget {
        w.message(3, &enc_budget(b));
    }
    w.opt_double(4, wl.value_latency);
    for d in &wl.data {
        w.message(5, &enc_data(d));
    }
    for s in &wl.services {
        w.message(6, &enc_service(s));
    }
    for j in &wl.jobs {
        w.message(7, &enc_job(j));
    }
    w
}

fn enc_price(p: &Price) -> Writer {
    let mut w = Writer::new();
    w.double(1, p.amount);
    w.string(2, &p.unit);
    w
}

fn enc_offer(o: &Offer) -> Writer {
    let mut w = Writer::new();
    w.string(1, &o.id);
    w.opt_string(2, &o.vendor);
    w.opt_string(3, &o.country);
    w.opt_uint(4, o.trust.map(|x| x as u64));
    for c in &o.classes {
        w.string_present(5, c);
    }
    if let Some(p) = &o.price {
        w.message(6, &enc_price(p));
    }
    if !o.resources.is_empty() {
        w.message(7, &enc_resources(&o.resources));
    }
    for f in &o.features {
        w.string_present(8, f);
    }
    w.opt_string(9, &o.subdivision);
    for j in &o.jurisdictions {
        w.string_present(10, j);
    }
    w.opt_string(11, &o.keys);
    for (r, ms) in &o.latency {
        let mut m = Writer::new();
        m.string(1, r);
        m.double(2, *ms);
        w.message(12, &m);
    }
    w
}

/// Encode a (checked, import-resolved) file as `bwk.v0.File`.
pub fn encode_file(file: &File) -> Vec<u8> {
    let mut w = Writer::new();
    w.string(1, crate::ir::VERSION);
    for i in &file.imports {
        w.string_present(2, &i.path);
    }
    for p in &file.policies {
        w.message(3, &enc_policy(p));
    }
    for wl in &file.workloads {
        w.message(4, &enc_workload(wl));
    }
    for o in &file.offers {
        w.message(5, &enc_offer(o));
    }
    w.buf
}

// --- encoders: plan.proto ----------------------------------------------------------

/// Encode feasibility plans as `bwk.plan.v0.Plan`.
pub fn encode_plans(plans: &[WorkloadPlan]) -> Vec<u8> {
    let mut w = Writer::new();
    w.string(1, crate::report::PLAN_VERSION);
    for p in plans {
        let blocking = p.blocking();
        let mut wp = Writer::new();
        wp.string(1, &p.workload);
        for n in &p.policies {
            wp.string_present(2, n);
        }
        wp.boolean(3, blocking.is_empty());
        for b in &blocking {
            wp.string_present(4, b);
        }
        for u in &p.units {
            let mut wu = Writer::new();
            wu.string(1, u.kind);
            wu.string(2, &u.name);
            wu.string(3, &u.class);
            wu.string(4, &u.effective_residency);
            wu.uint(5, u.effective_sensitivity as u64);
            wu.uint(6, u.min_trust as u64);
            wu.opt_int64(7, u.min_vendors);
            if !u.resources.is_empty() {
                wu.message(8, &enc_resources(&u.resources));
            }
            for f in u.feasible_ids() {
                wu.string_present(9, f);
            }
            wu.uint(10, u.distinct_feasible_vendors() as u64);
            for v in &u.verdicts {
                if v.feasible() {
                    continue;
                }
                let mut we = Writer::new();
                we.string(1, &v.offer_id);
                for e in &v.exclusions {
                    let mut wr = Writer::new();
                    wr.string(1, e.kind());
                    wr.string(2, &e.reason());
                    we.message(2, &wr);
                }
                wu.message(11, &we);
            }
            if let Some(r) = u.fails_closed() {
                wu.string_present(12, &r);
            }
            for f in &u.requires {
                wu.string_present(13, f);
            }
            wu.opt_string(14, &u.customer_keys);
            wu.opt_double(15, u.slo_p95_ms);
            for r in &u.slo_from {
                wu.string_present(16, r);
            }
            wp.message(5, &wu);
        }
        w.message(2, &wp);
    }
    w.buf
}

// --- decoder -------------------------------------------------------------------

#[derive(Debug)]
enum Val<'a> {
    Varint(u64),
    Fixed64(u64),
    /// Skipped: no BWK schema uses fixed32, but a decoder must still be
    /// able to step over one.
    Fixed32,
    Bytes(&'a [u8]),
}

fn read_varint(b: &[u8], p: &mut usize) -> Result<u64, String> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *b.get(*p).ok_or_else(|| "truncated varint".to_string())?;
        *p += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint too long".to_string());
        }
    }
}

fn fields(b: &[u8]) -> Result<Vec<(u32, Val<'_>)>, String> {
    let mut p = 0usize;
    let mut out = Vec::new();
    while p < b.len() {
        let tag = read_varint(b, &mut p)?;
        let field = (tag >> 3) as u32;
        let wt = (tag & 7) as u8;
        match wt {
            WT_VARINT => out.push((field, Val::Varint(read_varint(b, &mut p)?))),
            WT_FIXED64 => {
                let end = p.checked_add(8).ok_or("length overflow")?;
                let s = b.get(p..end).ok_or("truncated fixed64")?;
                p = end;
                let mut a = [0u8; 8];
                a.copy_from_slice(s);
                out.push((field, Val::Fixed64(u64::from_le_bytes(a))));
            }
            WT_LEN => {
                let len = read_varint(b, &mut p)? as usize;
                let end = p.checked_add(len).ok_or("length overflow")?;
                let s = b.get(p..end).ok_or("truncated bytes")?;
                p = end;
                out.push((field, Val::Bytes(s)));
            }
            WT_FIXED32 => {
                let end = p.checked_add(4).ok_or("length overflow")?;
                b.get(p..end).ok_or("truncated fixed32")?;
                p = end;
                out.push((field, Val::Fixed32));
            }
            other => return Err(format!("unsupported wire type {}", other)),
        }
    }
    Ok(out)
}

fn str_of(v: &Val) -> Result<String, String> {
    match v {
        Val::Bytes(b) => String::from_utf8(b.to_vec()).map_err(|_| "invalid utf-8 string".to_string()),
        _ => Err("expected a length-delimited field".to_string()),
    }
}
fn f64_of(v: &Val) -> Result<f64, String> {
    match v {
        Val::Fixed64(x) => Ok(f64::from_bits(*x)),
        _ => Err("expected a fixed64 field".to_string()),
    }
}
fn u64_of(v: &Val) -> Result<u64, String> {
    match v {
        Val::Varint(x) => Ok(*x),
        _ => Err("expected a varint field".to_string()),
    }
}
fn level_of(v: &Val) -> Result<u8, String> {
    let n = u64_of(v)?;
    if n > 3 {
        return Err(format!("level {} out of range 0..3", n));
    }
    Ok(n as u8)
}
fn bytes_of<'a>(v: &Val<'a>) -> Result<&'a [u8], String> {
    match v {
        Val::Bytes(b) => Ok(b),
        _ => Err("expected a length-delimited field".to_string()),
    }
}

fn dec_budget(b: &[u8]) -> Result<Budget, String> {
    let mut out = Budget { amount: 0.0, period: String::new() };
    for (f, v) in fields(b)? {
        match f {
            1 => out.amount = f64_of(&v)?,
            2 => out.period = str_of(&v)?,
            _ => {}
        }
    }
    Ok(out)
}

fn dec_resources(b: &[u8]) -> Result<Resources, String> {
    let mut r = Resources::default();
    for (f, v) in fields(b)? {
        match f {
            1 => r.vcpu = Some(f64_of(&v)?),
            2 => r.mem_gb = Some(f64_of(&v)?),
            3 => r.disk_gb = Some(f64_of(&v)?),
            4 => r.gpu = Some(f64_of(&v)?),
            _ => {}
        }
    }
    Ok(r)
}

fn dec_policy(b: &[u8]) -> Result<Policy, String> {
    let mut p = Policy {
        name: String::new(),
        residency: None,
        min_trust: None,
        deny_vendors: Vec::new(),
        deny_countries: Vec::new(),
        deny_jurisdictions: Vec::new(),
        keys: None,
        budget_cap: None,
        line: 0,
        col: 0,
    };
    for (f, v) in fields(b)? {
        match f {
            1 => p.name = str_of(&v)?,
            2 => p.residency = Some(str_of(&v)?),
            3 => p.min_trust = Some(level_of(&v)?),
            4 => p.deny_vendors.push(str_of(&v)?),
            5 => p.deny_countries.push(str_of(&v)?),
            6 => p.budget_cap = Some(dec_budget(bytes_of(&v)?)?),
            7 => p.deny_jurisdictions.push(str_of(&v)?),
            8 => p.keys = Some(str_of(&v)?),
            _ => {}
        }
    }
    Ok(p)
}

fn dec_data(b: &[u8]) -> Result<DataDecl, String> {
    let mut d = DataDecl { name: String::new(), sensitivity: None, residency: None, keys: None, line: 0, col: 0 };
    for (f, v) in fields(b)? {
        match f {
            1 => d.name = str_of(&v)?,
            2 => d.sensitivity = Some(level_of(&v)?),
            3 => d.residency = Some(str_of(&v)?),
            4 => d.keys = Some(str_of(&v)?),
            _ => {}
        }
    }
    Ok(d)
}

fn dec_cache(b: &[u8]) -> Result<CacheRule, String> {
    let mut c = CacheRule { path: String::new(), ttl_ms: None, line: 0, col: 0 };
    for (f, v) in fields(b)? {
        match f {
            1 => c.path = str_of(&v)?,
            2 => c.ttl_ms = Some(f64_of(&v)?),
            // Field 3 is a "no TTL" flag: a non-zero value clears whatever
            // field 2 set. A zero value fails the guard and falls through
            // to the catch-all below, which leaves `ttl_ms` as it was —
            // the same behaviour as the nested `if` this replaces.
            3 if u64_of(&v)? != 0 => c.ttl_ms = None,
            _ => {}
        }
    }
    Ok(c)
}

fn dec_service(b: &[u8]) -> Result<Service, String> {
    let mut s = Service {
        name: String::new(),
        class: String::new(),
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
        line: 0,
        col: 0,
    };
    for (f, v) in fields(b)? {
        match f {
            1 => s.name = str_of(&v)?,
            2 => s.class = str_of(&v)?,
            3 => s.image = Some(str_of(&v)?),
            4 => s.sensitivity = Some(level_of(&v)?),
            5 => s.residency = Some(str_of(&v)?),
            6 => s.slo_p95_ms = Some(f64_of(&v)?),
            7 => s.min_vendors = Some(u64_of(&v)? as i64),
            8 => s.depends_on.push(str_of(&v)?),
            9 => s.resources = dec_resources(bytes_of(&v)?)?,
            10 => s.value_latency = Some(f64_of(&v)?),
            11 => s.cache.push(dec_cache(bytes_of(&v)?)?),
            12 => s.requires.push(str_of(&v)?),
            13 => s.keys = Some(str_of(&v)?),
            14 => s.slo_from.push(str_of(&v)?),
            _ => {}
        }
    }
    Ok(s)
}

fn dec_job(b: &[u8]) -> Result<Job, String> {
    let mut j = Job {
        name: String::new(),
        class: String::new(),
        image: None,
        deadline_ms: None,
        checkpointable: None,
        sensitivity: None,
        residency: None,
        resources: Resources::default(),
        keys: None,
        line: 0,
        col: 0,
    };
    for (f, v) in fields(b)? {
        match f {
            1 => j.name = str_of(&v)?,
            2 => j.class = str_of(&v)?,
            3 => j.image = Some(str_of(&v)?),
            4 => j.deadline_ms = Some(f64_of(&v)?),
            5 => j.checkpointable = Some(u64_of(&v)? != 0),
            6 => j.sensitivity = Some(level_of(&v)?),
            7 => j.residency = Some(str_of(&v)?),
            8 => j.resources = dec_resources(bytes_of(&v)?)?,
            9 => j.keys = Some(str_of(&v)?),
            _ => {}
        }
    }
    Ok(j)
}

fn dec_workload(b: &[u8]) -> Result<Workload, String> {
    let mut w = Workload {
        name: String::new(),
        profile: None,
        budget: None,
        value_latency: None,
        data: Vec::new(),
        services: Vec::new(),
        jobs: Vec::new(),
        line: 0,
        col: 0,
    };
    for (f, v) in fields(b)? {
        match f {
            1 => w.name = str_of(&v)?,
            2 => w.profile = Some(str_of(&v)?),
            3 => w.budget = Some(dec_budget(bytes_of(&v)?)?),
            4 => w.value_latency = Some(f64_of(&v)?),
            5 => w.data.push(dec_data(bytes_of(&v)?)?),
            6 => w.services.push(dec_service(bytes_of(&v)?)?),
            7 => w.jobs.push(dec_job(bytes_of(&v)?)?),
            _ => {}
        }
    }
    Ok(w)
}

fn dec_price(b: &[u8]) -> Result<Price, String> {
    let mut p = Price { amount: 0.0, unit: String::new() };
    for (f, v) in fields(b)? {
        match f {
            1 => p.amount = f64_of(&v)?,
            2 => p.unit = str_of(&v)?,
            _ => {}
        }
    }
    Ok(p)
}

fn dec_offer(b: &[u8]) -> Result<Offer, String> {
    let mut o = Offer {
        id: String::new(),
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
        line: 0,
        col: 0,
    };
    for (f, v) in fields(b)? {
        match f {
            1 => o.id = str_of(&v)?,
            2 => o.vendor = Some(str_of(&v)?),
            3 => o.country = Some(str_of(&v)?),
            4 => o.trust = Some(level_of(&v)?),
            5 => o.classes.push(str_of(&v)?),
            6 => o.price = Some(dec_price(bytes_of(&v)?)?),
            7 => o.resources = dec_resources(bytes_of(&v)?)?,
            8 => o.features.push(str_of(&v)?),
            9 => o.subdivision = Some(str_of(&v)?),
            10 => o.jurisdictions.push(str_of(&v)?),
            11 => o.keys = Some(str_of(&v)?),
            12 => {
                let (mut r, mut ms) = (String::new(), 0.0);
                for (f, v) in fields(bytes_of(&v)?)? {
                    match f {
                        1 => r = str_of(&v)?,
                        2 => ms = f64_of(&v)?,
                        _ => {}
                    }
                }
                o.latency.push((r, ms));
            }
            _ => {}
        }
    }
    Ok(o)
}

/// Decode an `bwk.v0.File`. Source positions are not carried in the wire
/// format, so every decoded item has line 0.
pub fn decode_file(bytes: &[u8]) -> Result<File, String> {
    let mut file = File::default();
    let mut version: Option<String> = None;
    for (f, v) in fields(bytes)? {
        match f {
            1 => version = Some(str_of(&v)?),
            2 => file.imports.push(Import { path: str_of(&v)?, line: 0, col: 0 }),
            3 => file.policies.push(dec_policy(bytes_of(&v)?)?),
            4 => file.workloads.push(dec_workload(bytes_of(&v)?)?),
            5 => file.offers.push(dec_offer(bytes_of(&v)?)?),
            _ => {}
        }
    }
    match version.as_deref() {
        Some(v) if v == crate::ir::VERSION => Ok(file),
        Some(v) => Err(format!("unsupported IR version `{}` (expected `{}`)", v, crate::ir::VERSION)),
        None => Err("not an bwk.v0.File: missing version".to_string()),
    }
}
