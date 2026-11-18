//! `bailiwick prove`: answers that hold for every offer any provider could
//! ever make, not just the ones in today's catalog.
//!
//! Three questions, each per service and job:
//!
//! - **Can it run anywhere at all?** If no possible offer satisfies a unit,
//!   its requirements contradict each other, and no provider will ever be
//!   able to host it. Otherwise, a witness: an offer that would qualify.
//! - **Does it already satisfy a policy?** (`--implies gdpr`) Every offer the
//!   program allows is also allowed by the policy — so adopting the policy
//!   changes nothing. Otherwise, a counterexample: an offer the program
//!   allows and the policy forbids.
//! - **Is a change no looser than before?** (`--no-looser-than old.bwk`)
//!   Every offer the new version allows, the old version allowed too, for
//!   every unit both share. Otherwise, a counterexample.
//!
//! How it can speak for every possible offer: the rules only ever compare
//! an offer's attributes to constants the program mentions (a country in a
//! residency, a denied vendor, a trust floor, a resource minimum) or test
//! them for membership. Two offers that agree on every such comparison get
//! the same verdict. So it suffices to try one representative of each
//! class: every mentioned country and subdivision plus one that no rule
//! mentions, every trust level, every mentioned vendor plus a fresh one,
//! no jurisdiction or each denied one, and either key holder; with class,
//! resources, features and latency at the least the newer program accepts,
//! the hardest case for the older one. Counterexamples and witnesses are
//! real offers, re-checked by the evaluator.

use crate::ast::{Offer, Policy, Workload};
use crate::feasibility::{plan_workload, UnitPlan};
use crate::semantics::Residency;

/// The answer for one unit.
#[derive(Clone, Debug)]
pub struct Answer {
    /// "service `api`".
    pub unit: String,
    pub holds: bool,
    /// What was asked, in words.
    pub claim: String,
    /// A witness (satisfiable), or counterexamples (implication): one per
    /// distinct way the claim fails, each with why.
    pub examples: Vec<(Offer, Vec<String>)>,
    /// Anything else worth saying (e.g. redundancy lowered).
    pub notes: Vec<String>,
}

const FRESH: &str = "zz";

fn push<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

/// Every location that could make a difference to these residencies and
/// denial lists, plus locations no rule mentions.
fn locations(residencies: &[&Residency], countries: &[String]) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for r in residencies {
        if let Residency::Set(s) = r {
            for code in s {
                match code.split_once('-') {
                    Some((c, sub)) => {
                        push(&mut out, (c.to_string(), Some(sub.to_string())));
                        push(&mut out, (c.to_string(), None));
                        push(&mut out, (c.to_string(), Some(FRESH.to_string())));
                    }
                    None => {
                        push(&mut out, (code.to_string(), None));
                        push(&mut out, (code.to_string(), Some(FRESH.to_string())));
                    }
                }
            }
        }
    }
    for c in countries {
        push(&mut out, (c.clone(), None));
    }
    push(&mut out, (FRESH.to_string(), None));
    out
}

fn residency_of(u: &UnitPlan) -> &Residency {
    &u.residency
}

/// The least offer `u` accepts at a given location, trust, vendor, reach
/// and custody.
fn least_offer(id: usize, u: &UnitPlan, loc: &(String, Option<String>), trust: u8, vendor: &str, reach: &[String], customer: bool) -> Offer {
    Offer {
        id: format!("candidate-{}", id),
        vendor: Some(vendor.to_string()),
        country: Some(loc.0.clone()),
        subdivision: loc.1.clone(),
        trust: Some(trust),
        classes: vec![u.class.clone()],
        price: None,
        resources: u.resources.clone(),
        features: u.requires.clone(),
        jurisdictions: reach.to_vec(),
        keys: Some(if customer { "customer" } else { "vendor" }.to_string()),
        latency: match u.slo_p95_ms {
            Some(ms) => u.slo_from.iter().map(|r| (r.clone(), ms)).collect(),
            None => Vec::new(),
        },
        line: 0,
        col: 0,
    }
}

/// Every representative offer for comparing `new` against `old` (which may
/// be the same unit under more policies).
fn candidates(new: &UnitPlan, old: Option<&UnitPlan>, policies: &[&[Policy]]) -> Vec<Offer> {
    let mut countries = Vec::new();
    let mut vendors = Vec::new();
    let mut jurisdictions = Vec::new();
    for ps in policies {
        for p in ps.iter() {
            for c in &p.deny_countries {
                push(&mut countries, c.clone());
            }
            for v in &p.deny_vendors {
                push(&mut vendors, v.clone());
            }
            for j in &p.deny_jurisdictions {
                push(&mut jurisdictions, j.clone());
                push(&mut countries, j.clone());
            }
        }
    }
    push(&mut vendors, "any-vendor".to_string());
    let mut res = vec![residency_of(new)];
    if let Some(o) = old {
        res.push(residency_of(o));
    }
    let locs = locations(&res, &countries);
    let mut reaches: Vec<Vec<String>> = vec![Vec::new()];
    for j in &jurisdictions {
        reaches.push(vec![j.clone()]);
    }
    let mut out = Vec::new();
    for loc in &locs {
        for trust in 0..=3u8 {
            for v in &vendors {
                for reach in &reaches {
                    for customer in [true, false] {
                        out.push(least_offer(out.len(), new, loc, trust, v, reach, customer));
                    }
                }
            }
        }
    }
    out
}

fn units_of(w: &Workload, offers: &[Offer], policies: &[Policy]) -> Vec<UnitPlan> {
    plan_workload(w, offers, policies).units
}

fn name(u: &UnitPlan) -> String {
    format!("{} `{}`", u.kind, u.name)
}

/// Can each unit run on any possible offer?
pub fn satisfiable(w: &Workload, policies: &[Policy]) -> Vec<Answer> {
    let shells = units_of(w, &[], policies);
    shells
        .iter()
        .enumerate()
        .map(|(i, shell)| {
            let cands = candidates(shell, None, &[policies]);
            let plan = &units_of(w, &cands, policies)[i];
            let witness = plan.verdicts.iter().position(|v| v.feasible());
            match witness {
                Some(k) => Answer {
                    unit: name(shell),
                    holds: true,
                    claim: "some provider could host it".into(),
                    examples: vec![(cands[k].clone(), Vec::new())],
                    notes: Vec::new(),
                },
                None => {
                    // Every candidate failed: say which rules did it.
                    // Only the most permissive offers (top trust, own keys, no
                    // foreign reach) show what cannot be satisfied at all.
                    let mut kinds: Vec<&'static str> = Vec::new();
                    let permissive = |o: &Offer| o.trust == Some(3) && o.keys.as_deref() == Some("customer") && o.jurisdictions.is_empty();
                    for (v, _) in plan.verdicts.iter().zip(&cands).filter(|(_, o)| permissive(o)) {
                        for e in &v.exclusions {
                            push(&mut kinds, e.kind());
                        }
                    }
                    let mut notes = Vec::new();
                    if shell.residency.is_empty() {
                        notes.push(format!("its residency is empty ({})", shell.effective_residency));
                    } else {
                        notes.push(format!(
                            "no possible offer passes every rule: each fails at least one of {} (residency here: {})",
                            kinds.join(", "),
                            shell.effective_residency
                        ));
                    }
                    Answer {
                        unit: name(shell),
                        holds: false,
                        claim: "some provider could host it".into(),
                        examples: Vec::new(),
                        notes,
                    }
                }
            }
        })
        .collect()
}

/// Compare `new` (under `new_policies`) with `old` (under `old_policies`):
/// does every offer the new allows, the old allow too? Units are matched by
/// kind and name; a unit only in `new` is reported as new.
pub fn no_looser(new: &Workload, new_policies: &[Policy], old: &Workload, old_policies: &[Policy], claim: &str) -> Vec<Answer> {
    let new_shells = units_of(new, &[], new_policies);
    let old_shells = units_of(old, &[], old_policies);
    new_shells
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let Some(j) = old_shells.iter().position(|o| o.kind == n.kind && o.name == n.name) else {
                return Answer {
                    unit: name(n),
                    holds: true,
                    claim: claim.to_string(),
                    examples: Vec::new(),
                    notes: vec!["new: no earlier version to compare with".into()],
                };
            };
            let o = &old_shells[j];
            let cands = candidates(n, Some(o), &[new_policies, old_policies]);
            let np = &units_of(new, &cands, new_policies)[i];
            let op = &units_of(old, &cands, old_policies)[j];
            let mut notes = Vec::new();
            if let (Some(a), Some(b)) = (n.min_vendors, o.min_vendors) {
                if a < b {
                    notes.push(format!("redundancy lowered from {} vendors to {}", b, a));
                }
            } else if n.min_vendors.is_none() && o.min_vendors.is_some() {
                notes.push(format!("redundancy removed (was {} vendors)", o.min_vendors.unwrap_or(0)));
            }
            // Counterexamples: allowed by the new, excluded by the old. Keep
            // the clearest one for each distinct reason (fewest exclusions).
            let mut best: Vec<(Vec<&'static str>, usize)> = Vec::new();
            for (k, (a, b)) in np.verdicts.iter().zip(&op.verdicts).enumerate() {
                if a.feasible() && !b.feasible() {
                    let mut kinds: Vec<&'static str> = b.exclusions.iter().map(|e| e.kind()).collect();
                    kinds.dedup();
                    let new_single = kinds.len() == 1 && !best.iter().any(|(ks, _)| *ks == kinds);
                    if new_single || (best.is_empty() && kinds.len() > 1) {
                        best.push((kinds, k));
                    }
                }
            }
            if best.len() > 1 {
                best.retain(|(ks, _)| ks.len() == 1);
            }
            let examples: Vec<(Offer, Vec<String>)> = best
                .iter()
                .map(|(_, k)| (cands[*k].clone(), op.verdicts[*k].exclusions.iter().map(|e| plain(&e.reason())).collect()))
                .collect();
            Answer { unit: name(n), holds: notes.is_empty() && examples.is_empty(), claim: claim.to_string(), examples, notes }
        })
        .collect()
}

/// Does every offer the program allows also satisfy `extra`?
pub fn implies(w: &Workload, policies: &[Policy], extra: &[Policy]) -> Vec<Answer> {
    let mut with: Vec<Policy> = policies.to_vec();
    with.extend(extra.iter().cloned());
    let names: Vec<String> = extra.iter().map(|p| format!("`{}`", p.name)).collect();
    no_looser(w, policies, w, &with, &format!("everything it allows satisfies {}", names.join(", ")))
}

/// A reason, with the stand-in for "a country no rule mentions" spelled out.
fn plain(reason: &str) -> String {
    reason.replace(&format!("`{}`", FRESH), "`<any other>`").replace(&format!("-{}`", FRESH), "-<any other>`")
}

fn describe(o: &Offer) -> String {
    let loc = match &o.subdivision {
        Some(s) if s != FRESH => format!("{}-{}", o.country.as_deref().unwrap_or("?"), s),
        Some(_) => format!("{} (any subdivision)", o.country.as_deref().unwrap_or("?")),
        None => o.country.clone().unwrap_or_default(),
    };
    let loc = if loc.starts_with(FRESH) { "a country no rule mentions".to_string() } else { loc };
    let mut parts = vec![format!("in {}", loc), format!("trust T{}", o.trust.unwrap_or(0))];
    match o.vendor.as_deref() {
        Some("any-vendor") | None => {}
        Some(v) => parts.push(format!("vendor {}", v)),
    }
    if !o.jurisdictions.is_empty() {
        parts.push(format!("reachable by {} law", o.jurisdictions.join(", ")));
    }
    parts.push(if o.keys.as_deref() == Some("customer") { "customer-held keys".into() } else { "provider-held keys".into() });
    parts.join(", ")
}

/// A plain-text report; the second value is whether everything holds.
pub fn report(answers: &[Answer]) -> (String, bool) {
    let mut s = String::new();
    let mut all = true;
    for a in answers {
        all &= a.holds;
        s.push_str(&format!("  {} {}: {}\n", if a.holds { "proven  " } else { "REFUTED " }, a.unit, a.claim));
        for (o, why) in &a.examples {
            if a.holds {
                s.push_str(&format!("           e.g. an offer {}\n", describe(o)));
            } else {
                s.push_str(&format!("           counterexample: an offer {}\n", describe(o)));
                for w in why {
                    s.push_str(&format!("             {}\n", w));
                }
            }
        }
        for n in &a.notes {
            s.push_str(&format!("           {}\n", n));
        }
    }
    (s, all)
}

