//! Rendering of feasibility plans: a human-readable report and the plan
//! IR (JSON; schema in `spec/ir/plan.proto`). The JSON is what the
//! conformance suite compares, so its shape and ordering are fixed.

use crate::feasibility::{UnitPlan, WorkloadPlan};
use crate::json::Json;

pub const PLAN_VERSION: &str = "bwk-plan/0";

// --- text -------------------------------------------------------------------

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn unit_text(u: &UnitPlan, out: &mut String) {
    let feasible = u.feasible_ids();
    out.push_str(&format!(
        "  {} {} [{}]  residency={} sensitivity=S{} min_trust=T{}\n",
        u.kind, u.name, u.class, u.effective_residency, u.effective_sensitivity, u.min_trust
    ));
    if !u.resources.is_empty() {
        let parts: Vec<String> = u
            .resources
            .fields()
            .into_iter()
            .map(|(n, v, unit)| {
                if unit.is_empty() {
                    format!("{}>={}", n, num(v))
                } else {
                    format!("{}>={}{}", n, num(v), unit)
                }
            })
            .collect();
        out.push_str(&format!("    requires: {}\n", parts.join(" ")));
    }
    if !u.requires.is_empty() {
        out.push_str(&format!("    requires features: {}\n", u.requires.join(", ")));
    }
    if let Some(by) = &u.customer_keys {
        out.push_str(&format!("    keys: customer-held (required by {})\n", by));
    }
    if let (Some(ms), false) = (u.slo_p95_ms, u.slo_from.is_empty()) {
        out.push_str(&format!("    latency: measured p95 <= {} ms from {}\n", num(ms), u.slo_from.join(", ")));
    }
    if feasible.is_empty() {
        out.push_str("    feasible: (none)\n");
    } else {
        out.push_str(&format!(
            "    feasible: {}  ({} vendor(s))\n",
            feasible.join(", "),
            u.distinct_feasible_vendors()
        ));
    }
    for v in &u.verdicts {
        if !v.feasible() {
            let reasons: Vec<String> = v.exclusions.iter().map(|e| e.reason()).collect();
            out.push_str(&format!("    excluded {}: {}\n", v.offer_id, reasons.join("; ")));
        }
    }
    if let Some(n) = u.min_vendors {
        out.push_str(&format!("    requires >= {} vendor(s)\n", n));
    }
}

pub fn text(plans: &[WorkloadPlan]) -> String {
    let mut out = String::new();
    for p in plans {
        out.push_str(&format!("workload {}\n", p.workload));
        if !p.policies.is_empty() {
            out.push_str(&format!("  policies: {}\n", p.policies.join(", ")));
        }
        for u in &p.units {
            unit_text(u, &mut out);
        }
        let blocking = p.blocking();
        if blocking.is_empty() {
            out.push_str("  => deployable\n\n");
        } else {
            out.push_str("  => BLOCKED:\n");
            for b in blocking {
                out.push_str(&format!("     - {}\n", b));
            }
            out.push('\n');
        }
    }
    out
}

// --- json -------------------------------------------------------------------

fn unit_json(u: &UnitPlan) -> Json {
    let feasible: Vec<&str> = u.feasible_ids();
    let excluded: Vec<Json> = u
        .verdicts
        .iter()
        .filter(|v| !v.feasible())
        .map(|v| {
            Json::obj(vec![
                ("offer", Json::str(&v.offer_id)),
                (
                    "reasons",
                    Json::Arr(
                        v.exclusions
                            .iter()
                            .map(|e| {
                                Json::obj(vec![
                                    ("kind", Json::str(e.kind())),
                                    ("reason", Json::str(e.reason())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    Json::obj(vec![
        ("kind", Json::str(u.kind)),
        ("name", Json::str(&u.name)),
        ("class", Json::str(&u.class)),
        ("residency", Json::str(&u.effective_residency)),
        ("sensitivity", Json::Num(u.effective_sensitivity as f64)),
        ("min_trust", Json::Num(u.min_trust as f64)),
        ("min_vendors", Json::opt_i64(u.min_vendors)),
        ("resources", crate::ir::resources(&u.resources)),
        ("requires", Json::strs(&u.requires)),
        ("customer_keys", Json::opt_str(&u.customer_keys)),
        ("slo_p95_ms", Json::opt_f64(u.slo_p95_ms)),
        ("slo_from", Json::strs(&u.slo_from)),
        ("feasible", Json::strs(&feasible)),
        ("feasible_vendors", Json::Num(u.distinct_feasible_vendors() as f64)),
        ("excluded", Json::Arr(excluded)),
        (
            "fails_closed",
            match u.fails_closed() {
                Some(r) => Json::str(r),
                None => Json::Null,
            },
        ),
    ])
}

pub fn json(plans: &[WorkloadPlan]) -> Json {
    Json::obj(vec![
        ("version", Json::str(PLAN_VERSION)),
        (
            "workloads",
            Json::Arr(
                plans
                    .iter()
                    .map(|p| {
                        let blocking = p.blocking();
                        Json::obj(vec![
                            ("workload", Json::str(&p.workload)),
                            ("policies", Json::strs(&p.policies)),
                            ("deployable", Json::Bool(blocking.is_empty())),
                            ("blocking", Json::strs(&blocking)),
                            ("units", Json::Arr(p.units.iter().map(unit_json).collect())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

pub fn json_string(plans: &[WorkloadPlan]) -> String {
    crate::json::to_string_pretty(&json(plans))
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('<', "&lt;").replace('>', "&gt;")
}

/// A Markdown summary for pull-request comments and job summaries: one
/// table row per unit, and the reasons behind every exclusion folded away.
pub fn markdown(plans: &[WorkloadPlan]) -> String {
    let mut out = String::new();
    for p in plans {
        let blocking = p.blocking();
        out.push_str(&format!(
            "#### `{}`: {}\n\n",
            p.workload,
            if blocking.is_empty() { "deploys" } else { "**fails closed**" }
        ));
        if !p.policies.is_empty() {
            let names: Vec<String> = p.policies.iter().map(|n| format!("`{}`", n)).collect();
            out.push_str(&format!("Policies in force: {}\n\n", names.join(", ")));
        }
        out.push_str("| Unit | Must have | May run on | Excluded |\n|---|---|---|---|\n");
        for u in &p.units {
            let mut must = vec![format!("residency {}", u.effective_residency), format!("T{}+", u.min_trust)];
            if let Some(n) = u.min_vendors {
                must.push(format!("{} vendors", n));
            }
            if u.customer_keys.is_some() {
                must.push("customer-held keys".into());
            }
            if let (Some(ms), false) = (u.slo_p95_ms, u.slo_from.is_empty()) {
                must.push(format!("p95 ≤ {} ms from {}", num(ms), u.slo_from.join(", ")));
            }
            must.extend(u.requires.iter().cloned());
            let feasible = u.feasible_ids();
            let may = if feasible.is_empty() {
                "**nowhere**".to_string()
            } else if feasible.len() <= 6 {
                feasible.iter().map(|f| format!("`{}`", f)).collect::<Vec<_>>().join(", ")
            } else {
                format!(
                    "{} and {} more ({} vendors)",
                    feasible[..4].iter().map(|f| format!("`{}`", f)).collect::<Vec<_>>().join(", "),
                    feasible.len() - 4,
                    u.distinct_feasible_vendors()
                )
            };
            let excluded = u.verdicts.iter().filter(|v| !v.feasible()).count();
            out.push_str(&format!(
                "| {} `{}` | {} | {} | {} |\n",
                u.kind,
                u.name,
                md_escape(&must.join(" · ")),
                may,
                excluded
            ));
        }
        for b in &blocking {
            out.push_str(&format!("\n> **Blocked:** {}\n", md_escape(b)));
        }
        out.push_str("\n<details><summary>Why each offer was excluded</summary>\n\n");
        for u in &p.units {
            out.push_str(&format!("**{} `{}`**\n\n", u.kind, u.name));
            for v in u.verdicts.iter().filter(|v| !v.feasible()) {
                let reasons: Vec<String> = v.exclusions.iter().map(|e| md_escape(&e.reason())).collect();
                out.push_str(&format!("- `{}`: {}\n", v.offer_id, reasons.join("; ")));
            }
            out.push('\n');
        }
        out.push_str("</details>\n\n");
    }
    out
}
