//! `bailiwick prove` speaks for every possible offer. This tries to catch
//! it out: random programs and policies, and thousands of random concrete
//! offers each. Whenever a concrete offer is a counterexample, `prove` must
//! have refuted the claim; whenever `prove` claims a unit can run, a
//! witness exists; and every counterexample `prove` reports is real.

use bailiwick::ast::{Offer, Resources};
use bailiwick::prove;

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
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
    fn subset<'a>(&mut self, xs: &[&'a str], pct: u64) -> Vec<&'a str> {
        xs.iter().copied().filter(|_| self.chance(pct)).collect()
    }
}

const REGIONS: &[&str] = &["global", "eu", "eea", "us", "uk", "ca", "california", "quebec", "gdpr-adequate", "nordics"];
const COUNTRIES: &[&str] = &["de", "fr", "us", "ca", "gb", "jp", "no", "se", "ch", "br", "ad"];
const SUBS: &[(&str, &str)] = &[("us", "ca"), ("us", "ny"), ("ca", "qc"), ("ca", "on"), ("de", "by")];
const VENDORS: &[&str] = &["vendor-a", "vendor-b", "vendor-c", "vendor-d", "vendor-e"];
const JURIS: &[&str] = &["us", "cn", "de", "gb"];
const FROM: &[&str] = &["eu", "us", "apac"];
const CLASSES: &[&str] = &["backend-api", "database"];

fn policy(r: &mut Rng, name: &str) -> String {
    let mut s = format!("policy {} {{\n", name);
    if r.chance(60) {
        s += &format!("  residency = {}\n", r.pick(REGIONS));
    }
    if r.chance(40) {
        s += &format!("  min_trust = T{}\n", r.below(4));
    }
    let v = r.subset(VENDORS, 15);
    if !v.is_empty() {
        s += &format!("  deny vendors = [ {} ]\n", v.join(", "));
    }
    let c = r.subset(COUNTRIES, 10);
    if !c.is_empty() {
        s += &format!("  deny countries = [ {} ]\n", c.join(", "));
    }
    let j = r.subset(JURIS, 20);
    if !j.is_empty() {
        s += &format!("  deny jurisdictions = [ {} ]\n", j.join(", "));
    }
    if r.chance(15) {
        s += "  keys = customer\n";
    }
    if s.ends_with("{\n") {
        s += "  min_trust = T1\n";
    }
    s + "}\n"
}

fn workload(r: &mut Rng) -> String {
    let mut s = String::from("workload w {\n  data d {\n");
    if r.chance(50) {
        s += &format!("    residency = {}\n", r.pick(REGIONS));
    }
    s += &format!("    sensitivity = S{}\n", r.below(4));
    if r.chance(20) {
        s += "    keys = customer\n";
    }
    s += "  }\n";
    s += &format!("  service s : {} {{\n", r.pick(CLASSES));
    if r.chance(50) {
        s += &format!("    residency = {}\n", r.pick(REGIONS));
    }
    if r.chance(60) {
        s += "    depends_on = d\n";
    }
    if r.chance(40) {
        s += &format!("    slo.p95 <= {} ms from {}\n", [30, 60, 120][r.below(3)], r.pick(FROM));
    }
    if r.chance(40) {
        s += &format!("    resources {{ vcpu >= {} }}\n", 1 + r.below(8));
    }
    if r.chance(25) {
        s += "    requires = [ waf ]\n";
    }
    s += "  }\n";
    s += &format!("  job j : background-jobs {{\n    deadline = 1 h\n    sensitivity = S{}\n", r.below(4));
    if r.chance(50) {
        s += &format!("    residency = {}\n", r.pick(REGIONS));
    }
    s + "  }\n}\n"
}

fn offer(r: &mut Rng, i: usize) -> Offer {
    let (country, sub) = if r.chance(30) {
        let (c, s) = SUBS[r.below(SUBS.len())];
        (c.to_string(), Some(s.to_string()))
    } else {
        (r.pick(COUNTRIES).to_string(), None)
    };
    let mut classes: Vec<String> = r.subset(CLASSES, 70).into_iter().map(String::from).collect();
    if r.chance(70) {
        classes.push("background-jobs".into());
    }
    Offer {
        id: format!("o{}", i),
        vendor: Some(r.pick(VENDORS).into()),
        country: Some(country),
        subdivision: sub,
        trust: Some(r.below(4) as u8),
        classes,
        price: None,
        resources: Resources { vcpu: Some((1 + r.below(10)) as f64), ..Default::default() },
        features: r.subset(&["waf", "cache"], 50).into_iter().map(String::from).collect(),
        jurisdictions: r.subset(JURIS, 25).into_iter().map(String::from).collect(),
        keys: if r.chance(50) { Some("customer".into()) } else { None },
        latency: {
            let mut l = Vec::new();
            for f in FROM {
                if r.chance(60) {
                    l.push((f.to_string(), [20.0, 30.0, 60.0, 90.0, 120.0, 200.0][r.below(6)]));
                }
            }
            l
        },
        line: 0,
        col: 0,
    }
}

fn parse(src: &str) -> Option<bailiwick::File> {
    let (file, diags) = bailiwick::parse_and_check(src);
    // Programs `check` rejects (e.g. a residency contradicting a policy)
    // are not ones `prove` is asked about.
    if bailiwick::any_errors(&diags) {
        None
    } else {
        Some(file)
    }
}

fn feasible(file: &bailiwick::File, policies: &[bailiwick::Policy], o: &Offer) -> Vec<bool> {
    bailiwick::plan_workload(&file.workloads[0], std::slice::from_ref(o), policies)
        .units
        .iter()
        .map(|u| u.verdicts[0].feasible())
        .collect()
}

#[test]
fn prove_is_never_caught_out_by_a_concrete_offer() {
    let mut r = Rng(0x2545_f491_4f6c_dd1d);
    let (mut programs, mut refuted, mut proven, mut unsat) = (0, 0, 0, 0);
    while programs < 400 {
        let base = format!("{}{}", policy(&mut r, "base"), workload(&mut r));
        let extra_src = policy(&mut r, "extra");
        let (Some(file), Some(extra)) = (parse(&base), parse(&extra_src)) else { continue };
        programs += 1;
        let w = &file.workloads[0];
        let mut with = file.policies.clone();
        with.extend(extra.policies.clone());

        let implied = prove::implies(w, &file.policies, &extra.policies);
        let sat = prove::satisfiable(w, &file.policies);
        let offers: Vec<Offer> = (0..1500).map(|i| offer(&mut r, i)).collect();
        for (u, a) in implied.iter().enumerate() {
            let concrete = offers.iter().find(|o| feasible(&file, &file.policies, o)[u] && !feasible(&file, &with, o)[u]);
            if a.holds {
                proven += 1;
                assert!(concrete.is_none(), "`prove` claimed {} but {:?} is a counterexample\n{}{}", a.claim, concrete, base, extra_src);
            } else {
                refuted += 1;
                for (o, _) in &a.examples {
                    assert!(feasible(&file, &file.policies, o)[u] && !feasible(&file, &with, o)[u], "reported counterexample is not one: {:?}", o);
                }
            }
        }
        for (u, a) in sat.iter().enumerate() {
            if a.holds {
                let (o, _) = &a.examples[0];
                assert!(feasible(&file, &file.policies, o)[u], "witness does not qualify: {:?}", o);
            } else {
                unsat += 1;
                assert!(!offers.iter().any(|o| feasible(&file, &file.policies, o)[u]), "`prove` said unsatisfiable\n{}", base);
            }
        }
    }
    eprintln!("{} programs: {} implications proven, {} refuted, {} units unsatisfiable", programs, proven, refuted, unsat);
    assert!(proven > 50 && refuted > 50, "both outcomes exercised");
}

#[test]
fn a_tightening_change_is_proven_and_a_loosening_one_refuted() {
    let old = parse("workload w { service s : backend-api { residency = eu  slo.p95 <= 100 ms from eu  redundancy { vendors >= 2 } } }").unwrap();
    let loader = |src: &str| bailiwick::loader::load_source(std::path::Path::new("t.bwk"), src, Vec::new()).file;
    let tighter = loader(&format!("import \"eu-sovereign\"\n{}", "workload w { service s : backend-api { residency = eu  slo.p95 <= 50 ms from [ eu, us ]  redundancy { vendors >= 2 } } }"));
    let a = prove::no_looser(&tighter.workloads[0], &tighter.policies, &old.workloads[0], &old.policies, "");
    assert!(a[0].holds, "{:?}", a[0]);
    let looser = parse("workload w { service s : backend-api { residency = eea  slo.p95 <= 100 ms from eu  redundancy { vendors >= 1 } } }").unwrap();
    let b = prove::no_looser(&looser.workloads[0], &looser.policies, &old.workloads[0], &old.policies, "");
    assert!(!b[0].holds);
    assert!(b[0].notes.iter().any(|n| n.contains("redundancy lowered")));
    assert!(b[0].examples.iter().any(|(o, _)| matches!(o.country.as_deref(), Some("is") | Some("li") | Some("no"))), "{:?}", b[0].examples);
}
