//! The implementation agrees with the proven model.
//!
//! `formal/` holds Bailiwick's semantics in Lean 4 and proofs of what is
//! true of them (policies only narrow, their order does not matter, an
//! offer is excluded exactly when a rule fails, ...). Those proofs are
//! about the model; this test is what makes them about the code people run.
//!
//! It generates random, valid Bailiwick programs (policies, a workload
//! with data dependencies, offers), runs each through the real parser,
//! checker and feasibility evaluator, sends the same parsed inputs to the
//! model's executable, and requires the same verdict for every offer,
//! exclusion by exclusion, and the same deploy-or-not decision.
//!
//! Set `BWK_LEAN_MODEL` to the built model (`formal/.lake/build/bin/bwk-model`)
//! to run it; `BWK_LEAN_CASES` (default 5000) sets how many cases.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use bailiwick::cert;
use bailiwick::json::Json;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next() % den < num
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
    fn subset<'a>(&mut self, xs: &[&'a str], p: u64) -> Vec<&'a str> {
        xs.iter().copied().filter(|_| self.chance(p, 100)).collect()
    }
}

const REGIONS: &[&str] = &["global", "eu", "eea", "nordics", "us", "ca", "uk", "apac", "california", "quebec"];
const VENDORS: &[&str] = &["vendor-a", "vendor-b", "vendor-c", "vendor-d", "vendor-e"];
const CLASSES: &[&str] = &["backend-api", "database", "background-jobs", "media-assets"];
const FEATURES: &[&str] = &["cache", "waf", "http3"];
/// Locations: a country, optionally with a subdivision.
const LOCATIONS: &[(&str, Option<&str>)] = &[
    ("de", None),
    ("de", Some("by")),
    ("fr", None),
    ("nl", None),
    ("se", None),
    ("no", None),
    ("gb", None),
    ("jp", None),
    ("us", None),
    ("us", Some("ca")),
    ("us", Some("ny")),
    ("ca", None),
    ("ca", Some("qc")),
];
const COUNTRIES: &[&str] = &["de", "fr", "us", "ca", "gb", "jp", "no"];
/// Jurisdictions that may reach an operator: some are offer countries too.
const JURISDICTIONS: &[&str] = &["us", "cn", "de", "gb", "ru"];

/// Regions latency is measured from.
const LATENCY_REGIONS: &[&str] = &["eu", "us", "uk", "apac", "ca"];

/// A latency, from a small set so targets and measurements often coincide,
/// with an occasional fraction.
fn ms(r: &mut Rng) -> String {
    let v = [20, 35, 50, 50, 80, 120, 200][r.below(7)];
    if r.chance(15, 100) {
        format!("{}.5", v)
    } else {
        v.to_string()
    }
}

/// Sometimes `keys = customer`, sometimes an explicit `keys = vendor`.
fn keys(r: &mut Rng, src: &mut String, indent: &str, customer_pct: u64) {
    if r.chance(customer_pct, 100) {
        src.push_str(&format!("{}keys = customer\n", indent));
    } else if r.chance(10, 100) {
        src.push_str(&format!("{}keys = vendor\n", indent));
    }
}

fn tlevel(r: &mut Rng) -> String {
    format!("T{}", r.below(4))
}

/// Offers lean towards fitting, so the feasible side is exercised as hard
/// as the excluded one.
fn offer_tlevel(r: &mut Rng) -> String {
    format!("T{}", [0, 1, 2, 2, 3, 3, 3][r.below(7)])
}

fn resources_req(r: &mut Rng) -> String {
    let mut out = Vec::new();
    if r.chance(50, 100) {
        out.push(format!("vcpu >= {}", 1 + r.below(8)));
    }
    if r.chance(40, 100) {
        out.push(format!("mem >= {} GB", 1 + r.below(32)));
    }
    if r.chance(20, 100) {
        out.push(format!("disk >= {} GB", 10 * (1 + r.below(50))));
    }
    if r.chance(10, 100) {
        out.push(format!("gpu >= {}", 1 + r.below(2)));
    }
    if out.is_empty() {
        String::new()
    } else {
        format!("resources {{ {} }}", out.join("  "))
    }
}

/// One random, valid program: policies, one workload, offers.
fn program(r: &mut Rng) -> String {
    let mut src = String::new();
    for i in 0..[0, 0, 1, 1, 2, 3][r.below(6)] {
        src.push_str(&format!("policy p{} {{\n", i));
        if r.chance(60, 100) {
            src.push_str(&format!("  residency = {}\n", r.pick(REGIONS)));
        }
        if r.chance(40, 100) {
            src.push_str(&format!("  min_trust = {}\n", tlevel(r)));
        }
        let dv = r.subset(VENDORS, 15);
        if !dv.is_empty() {
            src.push_str(&format!("  deny vendors = [ {} ]\n", dv.join(", ")));
        }
        let dc = r.subset(COUNTRIES, 15);
        if !dc.is_empty() {
            src.push_str(&format!("  deny countries = [ {} ]\n", dc.join(", ")));
        }
        let dj = r.subset(JURISDICTIONS, 15);
        if !dj.is_empty() {
            src.push_str(&format!("  deny jurisdictions = [ {} ]\n", dj.join(", ")));
        }
        keys(r, &mut src, "  ", 12);
        src.push_str("}\n");
    }
    src.push_str("workload w {\n");
    let ndata = r.below(3);
    for i in 0..ndata {
        src.push_str(&format!("  data d{} {{\n", i));
        if r.chance(50, 100) {
            src.push_str(&format!("    sensitivity = S{}\n", r.below(4)));
        }
        if r.chance(50, 100) {
            src.push_str(&format!("    residency = {}\n", r.pick(REGIONS)));
        }
        keys(r, &mut src, "    ", 15);
        src.push_str("  }\n");
    }
    let class = r.pick(CLASSES);
    if r.chance(80, 100) {
        src.push_str(&format!("  service s : {} {{\n", class));
        if r.chance(50, 100) {
            src.push_str(&format!("    residency = {}\n", r.pick(REGIONS)));
        }
        if r.chance(50, 100) {
            src.push_str(&format!("    sensitivity = S{}\n", r.below(4)));
        }
        for i in 0..ndata {
            if r.chance(60, 100) {
                src.push_str(&format!("    depends_on = d{}\n", i));
            }
        }
        if r.chance(30, 100) {
            src.push_str(&format!("    redundancy {{ vendors >= {} }}\n", 1 + r.below(3)));
        }
        let req = resources_req(r);
        if !req.is_empty() {
            src.push_str(&format!("    {}\n", req));
        }
        let f = r.subset(FEATURES, 25);
        if !f.is_empty() {
            src.push_str(&format!("    requires = [ {} ]\n", f.join(", ")));
        }
        keys(r, &mut src, "    ", 10);
        if r.chance(45, 100) {
            let from = r.subset(LATENCY_REGIONS, 45);
            let from = if from.is_empty() { vec![r.pick(LATENCY_REGIONS)] } else { from };
            let target = ms(r);
            if from.len() == 1 && r.chance(50, 100) {
                src.push_str(&format!("    slo.p95 <= {} ms from {}\n", target, from[0]));
            } else {
                src.push_str(&format!("    slo.p95 <= {} ms from [ {} ]\n", target, from.join(", ")));
            }
        } else if r.chance(15, 100) {
            // A target with no users: it must rule nothing out.
            src.push_str(&format!("    slo.p95 <= {} ms\n", ms(r)));
        }
        src.push_str("  }\n");
    } else {
        src.push_str(&format!("  job j : {} {{\n    deadline = 2 h\n", class));
        if r.chance(50, 100) {
            src.push_str(&format!("    residency = {}\n", r.pick(REGIONS)));
        }
        if r.chance(50, 100) {
            src.push_str(&format!("    sensitivity = S{}\n", r.below(4)));
        }
        let req = resources_req(r);
        if !req.is_empty() {
            src.push_str(&format!("    {}\n", req));
        }
        keys(r, &mut src, "    ", 10);
        src.push_str("  }\n");
    }
    src.push_str("}\n");
    for i in 0..1 + r.below(6) {
        src.push_str(&format!("offer o{} {{\n", i));
        if r.chance(90, 100) {
            src.push_str(&format!("  vendor = {}\n", r.pick(VENDORS)));
        }
        if r.chance(92, 100) {
            let (c, sub) = LOCATIONS[r.below(LOCATIONS.len())];
            src.push_str(&format!("  country = {}\n", c));
            if let Some(s) = sub {
                src.push_str(&format!("  subdivision = {}\n", s));
            }
        }
        if r.chance(92, 100) {
            src.push_str(&format!("  trust = {}\n", offer_tlevel(r)));
        }
        let mut cls = r.subset(CLASSES, 35);
        if r.chance(75, 100) && !cls.contains(&class) {
            cls.push(class);
        }
        src.push_str(&format!("  classes = [ {} ]\n", cls.join(", ")));
        let mut res = Vec::new();
        if r.chance(90, 100) {
            res.push(format!("vcpu = {}", 4 + r.below(16)));
        }
        if r.chance(70, 100) {
            res.push(format!("mem = {} GB", 8 + r.below(64)));
        }
        if r.chance(50, 100) {
            res.push(format!("disk = {} GB", 10 * (1 + r.below(60))));
        }
        if r.chance(40, 100) {
            res.push(format!("gpu = {}", r.below(3)));
        }
        if !res.is_empty() {
            src.push_str(&format!("  resources {{ {} }}\n", res.join("  ")));
        }
        let f = r.subset(FEATURES, 70);
        if !f.is_empty() {
            src.push_str(&format!("  features = [ {} ]\n", f.join(", ")));
        }
        let j = r.subset(JURISDICTIONS, 20);
        if !j.is_empty() {
            src.push_str(&format!("  jurisdictions = [ {} ]\n", j.join(", ")));
        }
        // Customer custody about half the time, so both sides of rules 8
        // and 9 are exercised.
        keys(r, &mut src, "  ", 55);
        // Latency measured from some regions, often near the targets, so
        // rule 10's equal, above, below and unmeasured cases all occur.
        let measured = r.subset(LATENCY_REGIONS, 60);
        if !measured.is_empty() {
            let parts: Vec<String> = measured.iter().map(|g| format!("p95 from {} = {} ms", g, ms(r))).collect();
            src.push_str(&format!("  latency {{ {} }}\n", parts.join("  ")));
        }
        src.push_str("}\n");
    }
    src
}

/// The case as the model reads it, and the implementation's verdict.
fn case(src: &str) -> (Json, Json) {
    // The program must parse. Check errors are allowed on purpose: an
    // offer missing its country or trust, or a residency that contradicts a
    // policy, is reported by `check` *and* still evaluated (missing_field;
    // everything excluded, failing closed), and those are exactly the edge
    // cases worth comparing against the model.
    let parsed = bailiwick::parser::parse(src);
    let errors: Vec<_> = parsed.diags.iter().filter(|d| d.severity == bailiwick::Severity::Error).map(|d| d.msg.clone()).collect();
    assert!(errors.is_empty(), "the generator wrote a program that does not parse: {:?}\n{}", errors, src);
    let file = parsed.file;
    let w = &file.workloads[0];
    let plan = bailiwick::plan_workload(w, &file.offers, &file.policies);
    let unit = &plan.units[0];
    let model_input = cert::case(w, cert::units(w)[0], &file.offers, &file.policies);
    let verdict = Json::obj(vec![
        ("verdicts", Json::Arr(unit.verdicts.iter().map(|v| Json::Arr(v.exclusions.iter().map(|e| Json::str(cert::render(e))).collect())).collect())),
        ("deploys", Json::Bool(unit.fails_closed().is_none())),
    ]);
    (model_input, verdict)
}

#[test]
fn the_implementation_agrees_with_the_proven_model() {
    let Ok(model) = std::env::var("BWK_LEAN_MODEL") else {
        eprintln!("skipped: set BWK_LEAN_MODEL to formal/.lake/build/bin/bwk-model");
        return;
    };
    let n: usize = std::env::var("BWK_LEAN_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let mut child = Command::new(&model).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("run the model");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut offers, mut excluded, mut deployed) = (0usize, 0usize, 0usize);
    for i in 0..n {
        let src = program(&mut rng);
        let (input, expect) = case(&src);
        writeln!(stdin, "{}", bailiwick::json::to_string_compact(&input)).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let got = bailiwick::json::parse(line.trim()).unwrap_or_else(|e| panic!("model output is not JSON ({}): {}", e, line));
        assert_eq!(
            bailiwick::json::to_string_compact(&got),
            bailiwick::json::to_string_compact(&expect),
            "case {}: the implementation and the proven model disagree\nprogram:\n{}",
            i,
            src
        );
        let v = expect.get("verdicts").unwrap().as_arr().unwrap();
        offers += v.len();
        excluded += v.iter().filter(|x| !x.as_arr().unwrap().is_empty()).count();
        deployed += usize::from(expect.get("deploys").unwrap().as_bool().unwrap());
    }
    drop(stdin);
    let _ = child.wait();
    eprintln!("{} programs, {} offers ({} excluded), {} deployable: implementation and model agree on all of them", n, offers, excluded, deployed);
}
