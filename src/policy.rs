//! Policy composition. The rule is *policies only narrow*: combining any
//! set of policies with a workload can remove candidates, never add them.
//! Each field composes by a monotone operation —
//!
//!   residency   → intersection (of country sets)
//!   min_trust   → maximum
//!   deny lists  → union (vendors, countries, jurisdictions)
//!   keys        → customer if any policy asks for it
//!   budget caps → all caps apply (effectively the minimum)
//!
//! — so the result is order-independent and an auditor can state
//! "org policy ⊆ every workload" as a theorem rather than a hope.

use crate::ast::{Budget, Policy};
use crate::semantics::Residency;

#[derive(Clone, Debug, Default)]
pub struct Effective {
    /// Names of every policy that was composed, in order.
    pub names: Vec<String>,
    /// Intersection of all policy residencies; `None` = unconstrained.
    pub residency: Option<Residency>,
    /// Names of the policies that contributed a residency.
    pub residency_from: Vec<String>,
    /// Two policies whose residencies have an empty intersection.
    pub residency_conflict: Option<(String, String)>,
    /// Maximum trust floor across policies (0 when none set one).
    pub min_trust: u8,
    /// (vendor, policy name)
    pub deny_vendors: Vec<(String, String)>,
    /// (country, policy name)
    pub deny_countries: Vec<(String, String)>,
    /// (jurisdiction, policy name)
    pub deny_jurisdictions: Vec<(String, String)>,
    /// The first policy that requires customer-held keys, if any.
    pub customer_keys: Option<String>,
    /// (cap, policy name)
    pub budget_caps: Vec<(Budget, String)>,
}

impl Effective {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Display form of the effective residency, if any.
    pub fn residency_display(&self) -> Option<String> {
        self.residency.as_ref().map(|r| r.display())
    }

    /// Which policy denies this vendor, if any.
    pub fn vendor_denied_by(&self, vendor: &str) -> Option<&str> {
        self.deny_vendors
            .iter()
            .find(|(v, _)| v == vendor)
            .map(|(_, p)| p.as_str())
    }

    /// Which policy denies this country, if any.
    pub fn country_denied_by(&self, country: &str) -> Option<&str> {
        self.deny_countries
            .iter()
            .find(|(c, _)| c == country)
            .map(|(_, p)| p.as_str())
    }
}

/// Compose a set of policies into one effective policy. Unknown region
/// names are skipped here (and reported by `check`), so a typo cannot
/// silently empty the feasible set.
pub fn effective(policies: &[Policy]) -> Effective {
    let mut eff = Effective::default();
    let mut last_residency_policy: Option<String> = None;

    for p in policies {
        eff.names.push(p.name.clone());

        if let Some(name) = &p.residency {
            if let Some(r) = Residency::from_name(name) {
                let combined = match &eff.residency {
                    None => r,
                    Some(acc) => acc.intersect(&r),
                };
                if combined.is_empty() && eff.residency_conflict.is_none() {
                    if let Some(prev) = &last_residency_policy {
                        eff.residency_conflict = Some((prev.clone(), p.name.clone()));
                    }
                }
                eff.residency = Some(combined);
                eff.residency_from.push(p.name.clone());
                last_residency_policy = Some(p.name.clone());
            }
        }

        if let Some(t) = p.min_trust {
            if t > eff.min_trust {
                eff.min_trust = t;
            }
        }

        for v in &p.deny_vendors {
            if !eff.deny_vendors.iter().any(|(x, _)| x == v) {
                eff.deny_vendors.push((v.clone(), p.name.clone()));
            }
        }
        for c in &p.deny_countries {
            if !eff.deny_countries.iter().any(|(x, _)| x == c) {
                eff.deny_countries.push((c.clone(), p.name.clone()));
            }
        }

        for j in &p.deny_jurisdictions {
            if !eff.deny_jurisdictions.iter().any(|(x, _)| x == j) {
                eff.deny_jurisdictions.push((j.clone(), p.name.clone()));
            }
        }
        if crate::ast::customer_keys(&p.keys) && eff.customer_keys.is_none() {
            eff.customer_keys = Some(p.name.clone());
        }

        if let Some(b) = &p.budget_cap {
            eff.budget_caps.push((b.clone(), p.name.clone()));
        }
    }

    eff
}
