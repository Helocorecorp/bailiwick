//! Canonical formatting: re-serialize the AST in a fixed shape so
//! `bailiwick fmt` produces one blessed layout. Order is imports, policies,
//! workloads, offers. Durations print in the largest exact unit; money
//! prints with up to four decimals, trimmed; data sizes print in GB.

use crate::ast::*;

fn money(amount: f64) -> String {
    let s = format!("{:.4}", amount);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    format!("${}", s)
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn duration(ms: f64) -> String {
    let units: [(f64, &str); 5] = [
        (86_400_000.0, "d"),
        (3_600_000.0, "h"),
        (60_000.0, "m"),
        (1_000.0, "s"),
        (1.0, "ms"),
    ];
    for (scale, name) in units {
        if ms >= scale && (ms / scale).fract() == 0.0 {
            return format!("{} {}", (ms / scale) as i64, name);
        }
    }
    let s = format!("{:.3}", ms);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    format!("{} ms", s)
}

fn sens(n: u8) -> String {
    format!("S{}", n)
}
fn trust(n: u8) -> String {
    format!("T{}", n)
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `resources { vcpu >= 2  mem >= 4 GB }` (or with `=` for offers).
fn resources(r: &Resources, op: &str) -> String {
    let parts: Vec<String> = r
        .fields()
        .into_iter()
        .map(|(name, v, unit)| {
            if unit.is_empty() {
                format!("{} {} {}", name, op, num(v))
            } else {
                format!("{} {} {} {}", name, op, num(v), unit)
            }
        })
        .collect();
    format!("resources {{ {} }}", parts.join("  "))
}

/// Canonical form: dollars per 10 ms per 1,000,000 requests.
fn value_of(v: f64) -> String {
    format!(
        "value_of latency = {} per 10 ms per 1000000 requests",
        money(v * 10.0 * 1_000_000.0)
    )
}

pub fn format_file(file: &File) -> String {
    let mut out = String::new();
    let mut first = true;
    let mut sep = |out: &mut String| {
        if !first {
            out.push('\n');
        }
        first = false;
    };

    if !file.imports.is_empty() {
        sep(&mut out);
        for i in &file.imports {
            out.push_str(&format!("import {}\n", quote(&i.path)));
        }
    }

    for p in &file.policies {
        sep(&mut out);
        out.push_str(&format!("policy {} {{\n", p.name));
        if let Some(r) = &p.residency {
            out.push_str(&format!("  residency = {}\n", r));
        }
        if let Some(t) = p.min_trust {
            out.push_str(&format!("  min_trust = {}\n", trust(t)));
        }
        if !p.deny_vendors.is_empty() {
            out.push_str(&format!("  deny vendors   = [ {} ]\n", p.deny_vendors.join(", ")));
        }
        if !p.deny_countries.is_empty() {
            out.push_str(&format!(
                "  deny countries = [ {} ]\n",
                p.deny_countries.join(", ")
            ));
        }
        if !p.deny_jurisdictions.is_empty() {
            out.push_str(&format!(
                "  deny jurisdictions = [ {} ]\n",
                p.deny_jurisdictions.join(", ")
            ));
        }
        if let Some(k) = &p.keys {
            out.push_str(&format!("  keys = {}\n", k));
        }
        if let Some(b) = &p.budget_cap {
            out.push_str(&format!("  budget <= {} / {}\n", money(b.amount), b.period));
        }
        out.push_str("}\n");
    }

    for w in &file.workloads {
        sep(&mut out);
        out.push_str(&format!("workload {} {{\n", w.name));
        if let Some(p) = &w.profile {
            out.push_str(&format!("  profile = {}\n", p));
        }
        if let Some(b) = &w.budget {
            out.push_str(&format!("  budget  = {} / {}\n", money(b.amount), b.period));
        }
        if let Some(v) = w.value_latency {
            out.push_str(&format!("  {}\n", value_of(v)));
        }
        for d in &w.data {
            out.push_str(&format!("\n  data {} {{\n", d.name));
            if let Some(s) = d.sensitivity {
                out.push_str(&format!("    sensitivity = {}\n", sens(s)));
            }
            if let Some(r) = &d.residency {
                out.push_str(&format!("    residency   = {}\n", r));
            }
            out.push_str("  }\n");
        }
        for s in &w.services {
            out.push_str(&format!("\n  service {} : {} {{\n", s.name, s.class));
            if let Some(i) = &s.image {
                out.push_str(&format!("    image       = {}\n", quote(i)));
            }
            if let Some(v) = s.sensitivity {
                out.push_str(&format!("    sensitivity = {}\n", sens(v)));
            }
            if let Some(r) = &s.residency {
                out.push_str(&format!("    residency   = {}\n", r));
            }
            if let Some(ms) = s.slo_p95_ms {
                let from = match s.slo_from.len() {
                    0 => String::new(),
                    1 => format!(" from {}", s.slo_from[0]),
                    _ => format!(" from [ {} ]", s.slo_from.join(", ")),
                };
                out.push_str(&format!("    slo.p95    <= {}{}\n", duration(ms), from));
            }
            if let Some(v) = s.min_vendors {
                out.push_str(&format!("    redundancy {{ vendors >= {} }}\n", v));
            }
            if !s.resources.is_empty() {
                out.push_str(&format!("    {}\n", resources(&s.resources, ">=")));
            }
            for dep in &s.depends_on {
                out.push_str(&format!("    depends_on  = {}\n", dep));
            }
            if let Some(v) = s.value_latency {
                out.push_str(&format!("    {}\n", value_of(v)));
            }
            for c in &s.cache {
                match c.ttl_ms {
                    Some(ms) => out.push_str(&format!("    cache {} for {}\n", quote(&c.path), duration(ms))),
                    None => out.push_str(&format!("    cache {} never\n", quote(&c.path))),
                }
            }
            if !s.requires.is_empty() {
                out.push_str(&format!("    requires    = [ {} ]\n", s.requires.join(", ")));
            }
            out.push_str("  }\n");
        }
        for j in &w.jobs {
            out.push_str(&format!("\n  job {} : {} {{\n", j.name, j.class));
            if let Some(i) = &j.image {
                out.push_str(&format!("    image          = {}\n", quote(i)));
            }
            if let Some(ms) = j.deadline_ms {
                out.push_str(&format!("    deadline       = {}\n", duration(ms)));
            }
            if let Some(c) = j.checkpointable {
                out.push_str(&format!("    checkpointable = {}\n", c));
            }
            if let Some(v) = j.sensitivity {
                out.push_str(&format!("    sensitivity    = {}\n", sens(v)));
            }
            if let Some(r) = &j.residency {
                out.push_str(&format!("    residency      = {}\n", r));
            }
            if !j.resources.is_empty() {
                out.push_str(&format!("    {}\n", resources(&j.resources, ">=")));
            }
            out.push_str("  }\n");
        }
        out.push_str("}\n");
    }

    for o in &file.offers {
        sep(&mut out);
        out.push_str(&format!("offer {} {{\n", o.id));
        if let Some(v) = &o.vendor {
            out.push_str(&format!("  vendor   = {}\n", v));
        }
        if let Some(c) = &o.country {
            out.push_str(&format!("  country  = {}\n", c));
        }
        if let Some(s) = &o.subdivision {
            out.push_str(&format!("  subdivision = {}\n", s));
        }
        if let Some(t) = o.trust {
            out.push_str(&format!("  trust    = {}\n", trust(t)));
        }
        if !o.classes.is_empty() {
            out.push_str(&format!("  classes  = [ {} ]\n", o.classes.join(", ")));
        }
        if let Some(p) = &o.price {
            out.push_str(&format!("  price    = {} / {}\n", money(p.amount), p.unit));
        }
        if !o.resources.is_empty() {
            out.push_str(&format!("  {}\n", resources(&o.resources, "=")));
        }
        if !o.features.is_empty() {
            out.push_str(&format!("  features = [ {} ]\n", o.features.join(", ")));
        }
        if !o.jurisdictions.is_empty() {
            out.push_str(&format!("  jurisdictions = [ {} ]\n", o.jurisdictions.join(", ")));
        }
        if let Some(k) = &o.keys {
            out.push_str(&format!("  keys     = {}\n", k));
        }
        if !o.latency.is_empty() {
            let parts: Vec<String> = o.latency.iter().map(|(r, ms)| format!("p95 from {} = {}", r, duration(*ms))).collect();
            out.push_str(&format!("  latency {{ {} }}\n", parts.join("  ")));
        }
        out.push_str("}\n");
    }

    out
}
