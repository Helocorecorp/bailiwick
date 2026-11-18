//! Cross-entity checks that run after parsing: references resolve, regions
//! and profiles are known, classes are recognized, and policies are
//! consistent with the workloads they govern. Unit errors are already
//! reported by the parser, so this pass is about meaning, not syntax.

use crate::ast::*;
use crate::diag::Diag;
use crate::policy::{effective, Effective};
use crate::semantics::*;

fn check_region(diags: &mut Vec<Diag>, line: usize, col: usize, r: &str, what: &str) {
    if !region_known(r) {
        diags.push(Diag::error(
            line,
            col,
            format!(
                "unknown residency region `{}` in {} (use one of: {})",
                r,
                what,
                region_names()
            ),
        ));
    }
}

/// A unit's own residency must be compatible with the policies in force.
/// A workload that asks for `us` under an `eu` policy is a conflict we
/// report here, visibly, rather than letting it fail silently at plan time.
fn check_residency_vs_policy(
    diags: &mut Vec<Diag>,
    eff: &Effective,
    line: usize,
    col: usize,
    r: &str,
    what: &str,
) {
    let own = match Residency::from_name(r) {
        Some(x) => x,
        None => return, // unknown: reported by check_region
    };
    if let Some(pr) = &eff.residency {
        if own.intersect(pr).is_empty() {
            diags.push(Diag::error(
                line,
                col,
                format!(
                    "{} declares residency `{}`, which conflicts with residency `{}` required by policy {}",
                    what,
                    r,
                    pr.display(),
                    eff.residency_from
                        .iter()
                        .map(|n| format!("`{}`", n))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
}

fn check_resources(diags: &mut Vec<Diag>, line: usize, col: usize, r: &Resources, what: &str) {
    for (name, v, _) in r.fields() {
        if v < 0.0 {
            diags.push(Diag::error(
                line,
                col,
                format!("{} resources.{} must not be negative", what, name),
            ));
        }
    }
}

fn check_keys(diags: &mut Vec<Diag>, line: usize, col: usize, keys: &Option<String>, what: &str) {
    if let Some(k) = keys {
        if k != "customer" && k != "vendor" {
            diags.push(Diag::error(
                line,
                col,
                format!("{} `keys = {}` is not defined (use `customer` or `vendor`)", what, k),
            ));
        }
    }
}

fn check_jurisdictions(diags: &mut Vec<Diag>, line: usize, col: usize, js: &[String], what: &str) {
    for j in js {
        if !(j.len() == 2 && j.chars().all(|c| c.is_ascii_lowercase())) {
            diags.push(Diag::warning(
                line,
                col,
                format!("{} jurisdiction `{}` is not a lowercase ISO 3166-1 alpha-2 code", what, j),
            ));
        }
    }
}

/// Latency: where users are, and what offers measured.
fn check_latency(diags: &mut Vec<Diag>, file: &File) {
    for w in &file.workloads {
        for s in &w.services {
            if s.slo_p95_ms.is_some() && s.slo_from.is_empty() {
                diags.push(Diag::warning(
                    s.line,
                    s.col,
                    format!(
                        "service `{}`: `slo.p95` has no `from`, so it rules out no offer; write `slo.p95 <= ... from <region>` (where your users are) to make it a rule",
                        s.name
                    ),
                ));
            }
            for r in &s.slo_from {
                if !region_known(r) || r == "global" {
                    diags.push(Diag::error(
                        s.line,
                        s.col,
                        format!("service `{}`: `from {}` is not a region (known: {})", s.name, r, region_names()),
                    ));
                }
            }
        }
    }
    for o in &file.offers {
        for (r, ms) in &o.latency {
            if !region_known(r) || r == "global" {
                diags.push(Diag::warning(o.line, o.col, format!("offer `{}`: latency from unknown region `{}`", o.id, r)));
            }
            if *ms < 0.0 {
                diags.push(Diag::error(o.line, o.col, format!("offer `{}`: latency from `{}` must not be negative", o.id, r)));
            }
        }
    }
}

/// `keys` and jurisdiction lists, everywhere they may appear.
fn check_custody(diags: &mut Vec<Diag>, file: &File) {
    for p in &file.policies {
        let what = format!("policy `{}`", p.name);
        check_keys(diags, p.line, p.col, &p.keys, &what);
        check_jurisdictions(diags, p.line, p.col, &p.deny_jurisdictions, &what);
    }
    for w in &file.workloads {
        for d in &w.data {
            check_keys(diags, d.line, d.col, &d.keys, &format!("data `{}`", d.name));
        }
        for s in &w.services {
            check_keys(diags, s.line, s.col, &s.keys, &format!("service `{}`", s.name));
        }
        for j in &w.jobs {
            check_keys(diags, j.line, j.col, &j.keys, &format!("job `{}`", j.name));
        }
    }
    for o in &file.offers {
        let what = format!("offer `{}`", o.id);
        check_keys(diags, o.line, o.col, &o.keys, &what);
        check_jurisdictions(diags, o.line, o.col, &o.jurisdictions, &what);
    }
}

pub fn check(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();
    check_custody(&mut diags, file);
    check_latency(&mut diags, file);

    // --- policies ----------------------------------------------------------
    for p in &file.policies {
        if let Some(r) = &p.residency {
            check_region(&mut diags, p.line, p.col, r, &format!("policy `{}`", p.name));
        }
        if p.residency.is_none()
            && p.min_trust.is_none()
            && p.deny_vendors.is_empty()
            && p.deny_countries.is_empty()
            && p.deny_jurisdictions.is_empty()
            && p.keys.is_none()
            && p.budget_cap.is_none()
        {
            diags.push(Diag::warning(
                p.line,
                p.col,
                format!("policy `{}` constrains nothing", p.name),
            ));
        }
    }
    let eff = effective(&file.policies);
    if let Some((a, b)) = &eff.residency_conflict {
        diags.push(Diag::error(
            0,
            0,
            format!(
                "policies `{}` and `{}` require residencies with no overlap; nothing can be feasible",
                a, b
            ),
        ));
    }

    // --- workloads ---------------------------------------------------------
    for w in &file.workloads {
        if let Some(p) = &w.profile {
            if !profile_known(p) {
                diags.push(Diag::error(
                    w.line,
                    w.col,
                    format!(
                        "unknown profile `{}` (use cheapest, balanced, fastest, sovereign, or green)",
                        p
                    ),
                ));
            }
        }

        // Budget against policy caps.
        if let Some(b) = &w.budget {
            for (cap, pname) in &eff.budget_caps {
                if cap.period == b.period {
                    if b.amount > cap.amount {
                        diags.push(Diag::error(
                            w.line,
                            w.col,
                            format!(
                                "workload `{}` budget ${} / {} exceeds the ${} / {} cap set by policy `{}`",
                                w.name, b.amount, b.period, cap.amount, cap.period, pname
                            ),
                        ));
                    }
                } else {
                    diags.push(Diag::warning(
                        w.line,
                        w.col,
                        format!(
                            "cannot compare workload `{}` budget per `{}` with policy `{}` cap per `{}`",
                            w.name, b.period, pname, cap.period
                        ),
                    ));
                }
            }
        }

        for d in &w.data {
            if let Some(r) = &d.residency {
                let what = format!("data `{}`", d.name);
                check_region(&mut diags, d.line, d.col, r, &what);
                check_residency_vs_policy(&mut diags, &eff, d.line, d.col, r, &what);
            }
        }

        for s in &w.services {
            let what = format!("service `{}`", s.name);
            if !class_known(&s.class) {
                diags.push(Diag::error(
                    s.line,
                    s.col,
                    format!("unknown asset class `{}` in {}", s.class, what),
                ));
            }
            if let Some(r) = &s.residency {
                check_region(&mut diags, s.line, s.col, r, &what);
                check_residency_vs_policy(&mut diags, &eff, s.line, s.col, r, &what);
            }
            if let Some(v) = s.min_vendors {
                if v < 1 {
                    diags.push(Diag::error(
                        s.line,
                        s.col,
                        format!("redundancy vendors must be >= 1, found {}", v),
                    ));
                }
            }
            check_resources(&mut diags, s.line, s.col, &s.resources, &what);
            let mut seen_paths: Vec<&str> = Vec::new();
            for c in &s.cache {
                if !c.path.starts_with('/') {
                    diags.push(Diag::error(
                        c.line,
                        c.col,
                        format!("{} cache path `{}` must start with `/`", what, c.path),
                    ));
                }
                if seen_paths.contains(&c.path.as_str()) {
                    diags.push(Diag::warning(
                        c.line,
                        c.col,
                        format!("{} has more than one cache rule for `{}`", what, c.path),
                    ));
                }
                seen_paths.push(&c.path);
            }
            for f in &s.requires {
                if !feature_known(f) {
                    diags.push(Diag::warning(
                        s.line,
                        s.col,
                        format!(
                            "{} requires unknown feature `{}` (known: {})",
                            what,
                            f,
                            FEATURES.join(", ")
                        ),
                    ));
                }
            }
            for dep in &s.depends_on {
                match w.data.iter().find(|d| &d.name == dep) {
                    None => diags.push(Diag::error(
                        s.line,
                        s.col,
                        format!(
                            "{} depends_on `{}`, which is not a data block in this workload",
                            what, dep
                        ),
                    )),
                    Some(d) => {
                        // A service and its data must be co-locatable.
                        if let (Some(sr), Some(dr)) = (&s.residency, &d.residency) {
                            if let (Some(a), Some(b)) =
                                (Residency::from_name(sr), Residency::from_name(dr))
                            {
                                if a.intersect(&b).is_empty() {
                                    diags.push(Diag::error(
                                        s.line,
                                        s.col,
                                        format!(
                                            "{} (residency `{}`) depends on data `{}` (residency `{}`); no region satisfies both",
                                            what, sr, d.name, dr
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        for j in &w.jobs {
            let what = format!("job `{}`", j.name);
            if !class_known(&j.class) {
                diags.push(Diag::error(
                    j.line,
                    j.col,
                    format!("unknown asset class `{}` in {}", j.class, what),
                ));
            }
            if let Some(r) = &j.residency {
                check_region(&mut diags, j.line, j.col, r, &what);
                check_residency_vs_policy(&mut diags, &eff, j.line, j.col, r, &what);
            }
            check_resources(&mut diags, j.line, j.col, &j.resources, &what);
        }

        // Duplicate names within a workload.
        let mut names: Vec<&str> = Vec::new();
        for d in &w.data {
            names.push(&d.name);
        }
        for s in &w.services {
            names.push(&s.name);
        }
        for j in &w.jobs {
            names.push(&j.name);
        }
        for i in 0..names.len() {
            for k in (i + 1)..names.len() {
                if names[i] == names[k] {
                    diags.push(Diag::error(
                        w.line,
                        w.col,
                        format!("duplicate name `{}` in workload `{}`", names[i], w.name),
                    ));
                }
            }
        }
    }

    // --- offers ------------------------------------------------------------
    let mut seen_ids: Vec<&str> = Vec::new();
    for o in &file.offers {
        if seen_ids.contains(&o.id.as_str()) {
            diags.push(Diag::error(
                o.line,
                o.col,
                format!("duplicate offer id `{}`", o.id),
            ));
        }
        seen_ids.push(&o.id);
        if o.country.is_none() {
            diags.push(Diag::error(
                o.line,
                o.col,
                format!("offer `{}` is missing `country`", o.id),
            ));
        }
        if o.trust.is_none() {
            diags.push(Diag::error(
                o.line,
                o.col,
                format!("offer `{}` is missing `trust`", o.id),
            ));
        }
        for c in &o.classes {
            if !class_known(c) {
                diags.push(Diag::warning(
                    o.line,
                    o.col,
                    format!("offer `{}` lists unknown class `{}`", o.id, c),
                ));
            }
        }
        check_resources(&mut diags, o.line, o.col, &o.resources, &format!("offer `{}`", o.id));
        for f in &o.features {
            if !feature_known(f) {
                diags.push(Diag::warning(
                    o.line,
                    o.col,
                    format!(
                        "offer `{}` lists unknown feature `{}` (known: {})",
                        o.id,
                        f,
                        FEATURES.join(", ")
                    ),
                ));
            }
        }
        if let Some(sub) = &o.subdivision {
            let ok = (1..=3).contains(&sub.len()) && sub.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            if !ok {
                diags.push(Diag::warning(
                    o.line,
                    o.col,
                    format!(
                        "offer `{}` subdivision `{}` is not a lowercase ISO 3166-2 code part (e.g. `ca`, `qc`)",
                        o.id, sub
                    ),
                ));
            }
        }
    }

    diags
}
