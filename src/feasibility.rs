//! The normative feasibility evaluator. For each routable unit (service
//! or job) it derives the effective residency, sensitivity, resource needs
//! and required edge features — folding in data dependencies and the
//! policies in force — then tests every offer against ten rules and
//! records, for excluded offers, exactly why. This is the trusted core:
//! the part the standard pins down and every implementation must
//! reproduce row for row.

use crate::ast::*;
use crate::policy::{effective, Effective};
use crate::semantics::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Exclusion {
    Capability { class: String },
    /// `subnational` is true when the residency distinguishes subdivisions;
    /// `needs_subdivision` when the offer's country is relevant but its
    /// subdivision is unknown.
    Residency {
        country: String,
        subdivision: Option<String>,
        region: String,
        subnational: bool,
        needs_subdivision: bool,
    },
    Trust { have: u8, need: u8 },
    PolicyVendor { vendor: String, policy: String },
    PolicyCountry { country: String, policy: String },
    /// `have` is None when the offer does not declare the resource at all.
    Resource { resource: String, unit: String, need: f64, have: Option<f64> },
    Feature { feature: String },
    /// The offer is within reach of a jurisdiction a policy denies, and the
    /// provider holds the keys: that jurisdiction could compel plaintext.
    Jurisdiction { jurisdiction: String, policy: String },
    /// Customer-held keys are required (by `required_by`) and the provider
    /// holds them.
    KeyCustody { required_by: String },
    /// The service's users are in `from`, and the offer's measured p95
    /// from there (`None`: not measured) misses the SLO.
    Latency { from: String, need_ms: f64, have_ms: Option<f64> },
    MissingField { field: String },
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn with_unit(v: f64, unit: &str) -> String {
    if unit.is_empty() {
        num(v)
    } else {
        format!("{} {}", num(v), unit)
    }
}

impl Exclusion {
    pub fn reason(&self) -> String {
        match self {
            Exclusion::Capability { class } => {
                format!("does not offer class `{}`", class)
            }
            Exclusion::Residency { country, subdivision, region, subnational, needs_subdivision } => {
                if *subnational {
                    let code = match subdivision {
                        Some(s) => format!("{}-{}", country, s),
                        None => country.clone(),
                    };
                    if *needs_subdivision {
                        format!(
                            "location `{}` is outside residency `{}` (subdivision unknown)",
                            code, region
                        )
                    } else {
                        format!("location `{}` is outside residency `{}`", code, region)
                    }
                } else {
                    format!("country `{}` is outside residency `{}`", country, region)
                }
            }
            Exclusion::Trust { have, need } => {
                format!("trust T{} is below the required T{}", have, need)
            }
            Exclusion::PolicyVendor { vendor, policy } => {
                format!("vendor `{}` is denied by policy `{}`", vendor, policy)
            }
            Exclusion::PolicyCountry { country, policy } => {
                format!("country `{}` is denied by policy `{}`", country, policy)
            }
            Exclusion::Resource { resource, unit, need, have } => match have {
                None => format!(
                    "does not declare resources.{}; {} required",
                    resource,
                    with_unit(*need, unit)
                ),
                Some(h) => format!(
                    "resources.{} is {}, below the required {}",
                    resource,
                    with_unit(*h, unit),
                    with_unit(*need, unit)
                ),
            },
            Exclusion::Feature { feature } => {
                format!("does not provide feature `{}`", feature)
            }
            Exclusion::Jurisdiction { jurisdiction, policy } => format!(
                "within reach of jurisdiction `{}`, denied by policy `{}`, and the provider holds the keys",
                jurisdiction, policy
            ),
            Exclusion::KeyCustody { required_by } => {
                format!("the provider holds the keys; {} requires customer-held keys", required_by)
            }
            Exclusion::Latency { from, need_ms, have_ms } => match have_ms {
                None => format!(
                    "no measured p95 from `{}`; the SLO needs {} ms or less from there",
                    from,
                    num(*need_ms)
                ),
                Some(h) => format!(
                    "p95 from `{}` is {} ms, above the {} ms SLO",
                    from,
                    num(*h),
                    num(*need_ms)
                ),
            },
            Exclusion::MissingField { field } => {
                format!("offer is missing `{}`", field)
            }
        }
    }

    /// A short machine-readable tag for the IR.
    pub fn kind(&self) -> &'static str {
        match self {
            Exclusion::Capability { .. } => "capability",
            Exclusion::Residency { .. } => "residency",
            Exclusion::Trust { .. } => "trust",
            Exclusion::PolicyVendor { .. } => "policy_vendor",
            Exclusion::PolicyCountry { .. } => "policy_country",
            Exclusion::Resource { .. } => "resource",
            Exclusion::Feature { .. } => "feature",
            Exclusion::Jurisdiction { .. } => "jurisdiction",
            Exclusion::KeyCustody { .. } => "key_custody",
            Exclusion::Latency { .. } => "latency",
            Exclusion::MissingField { .. } => "missing_field",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OfferVerdict {
    pub offer_id: String,
    pub vendor: Option<String>,
    /// Empty when the offer is feasible.
    pub exclusions: Vec<Exclusion>,
}

impl OfferVerdict {
    pub fn feasible(&self) -> bool {
        self.exclusions.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct UnitPlan {
    /// "service" or "job".
    pub kind: &'static str,
    pub name: String,
    pub class: String,
    /// Display form of `residency`.
    pub effective_residency: String,
    pub residency: Residency,
    pub effective_sensitivity: u8,
    pub min_trust: u8,
    pub min_vendors: Option<i64>,
    /// Resource minimums the unit requires.
    pub resources: Resources,
    /// Edge features the unit requires (explicit plus implicit `cache`).
    pub requires: Vec<String>,
    /// What requires customer-held keys (e.g. "data `patients`"), if anything.
    pub customer_keys: Option<String>,
    /// The p95 SLO in milliseconds, and where it is measured from. With
    /// regions, it is a placement rule.
    pub slo_p95_ms: Option<f64>,
    pub slo_from: Vec<String>,
    pub verdicts: Vec<OfferVerdict>,
}

impl UnitPlan {
    pub fn feasible_ids(&self) -> Vec<&str> {
        self.verdicts
            .iter()
            .filter(|v| v.feasible())
            .map(|v| v.offer_id.as_str())
            .collect()
    }

    pub fn distinct_feasible_vendors(&self) -> usize {
        let mut seen: Vec<&str> = Vec::new();
        for v in &self.verdicts {
            if v.feasible() {
                if let Some(vendor) = &v.vendor {
                    if !seen.contains(&vendor.as_str()) {
                        seen.push(vendor);
                    }
                }
            }
        }
        seen.len()
    }

    /// A unit fails closed if it has no feasible offer, or cannot meet its
    /// vendor-redundancy requirement.
    pub fn fails_closed(&self) -> Option<String> {
        if self.feasible_ids().is_empty() {
            return Some(format!("no feasible offer for {} `{}`", self.kind, self.name));
        }
        if let Some(n) = self.min_vendors {
            let have = self.distinct_feasible_vendors() as i64;
            if have < n {
                return Some(format!(
                    "{} `{}` needs {} vendors but only {} feasible vendor(s) exist",
                    self.kind, self.name, n, have
                ));
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
pub struct WorkloadPlan {
    pub workload: String,
    /// Names of the policies that were in force.
    pub policies: Vec<String>,
    pub units: Vec<UnitPlan>,
}

impl WorkloadPlan {
    /// The workload deploys only if no unit fails closed: the only way to
    /// not deploy is an empty feasible set.
    pub fn blocking(&self) -> Vec<String> {
        self.units.iter().filter_map(|u| u.fails_closed()).collect()
    }
}

/// What a unit asks of every offer, once dependencies and policies are folded in.
struct Need<'a> {
    class: &'a str,
    residency: &'a Residency,
    trust: u8,
    resources: &'a Resources,
    features: &'a [String],
    /// What requires customer-held keys, if anything.
    customer_keys: Option<&'a str>,
    /// The p95 SLO, and the regions it must be met from.
    slo: Option<f64>,
    slo_from: &'a [String],
}

fn eval_offer(offer: &Offer, need: &Need, eff: &Effective) -> OfferVerdict {
    let Need { class, residency, trust: need_trust, resources: needs, features, customer_keys, slo, slo_from } = *need;
    let mut exclusions = Vec::new();

    // 1. Capability.
    if !offer.classes.iter().any(|c| c == class) {
        exclusions.push(Exclusion::Capability { class: class.to_string() });
    }

    // 2–3. Residency, then policy country denial.
    match &offer.country {
        None => exclusions.push(Exclusion::MissingField { field: "country".into() }),
        Some(country) => {
            if !residency.contains(country, offer.subdivision.as_deref()) {
                let subnational = residency.is_subnational();
                exclusions.push(Exclusion::Residency {
                    country: country.clone(),
                    subdivision: offer.subdivision.clone(),
                    region: residency.display(),
                    subnational,
                    needs_subdivision: subnational
                        && offer.subdivision.is_none()
                        && residency.has_subdivisions_of(country),
                });
            }
            if let Some(p) = eff.country_denied_by(country) {
                exclusions.push(Exclusion::PolicyCountry {
                    country: country.clone(),
                    policy: p.to_string(),
                });
            }
        }
    }

    // 4. Trust.
    match offer.trust {
        None => exclusions.push(Exclusion::MissingField { field: "trust".into() }),
        Some(have) => {
            if have < need_trust {
                exclusions.push(Exclusion::Trust { have, need: need_trust });
            }
        }
    }

    // 5. Policy vendor denial.
    if let Some(vendor) = &offer.vendor {
        if let Some(p) = eff.vendor_denied_by(vendor) {
            exclusions.push(Exclusion::PolicyVendor {
                vendor: vendor.clone(),
                policy: p.to_string(),
            });
        }
    }

    // 6. Resources: every required minimum must be declared and met.
    for (name, need, unit) in needs.fields() {
        let have = offer.resources.get(name);
        let ok = matches!(have, Some(h) if h >= need);
        if !ok {
            exclusions.push(Exclusion::Resource {
                resource: name.to_string(),
                unit: unit.to_string(),
                need,
                have,
            });
        }
    }

    // 7. Edge features: every required feature must be provided.
    for f in features {
        if !offer.features.iter().any(|x| x == f) {
            exclusions.push(Exclusion::Feature { feature: f.clone() });
        }
    }

    // 8. Jurisdiction: no denied jurisdiction may reach an offer whose
    // provider holds the keys. An offer is within reach of the law of its
    // own country and of every jurisdiction it declares.
    let held = customer_keys_held(offer);
    for (j, p) in &eff.deny_jurisdictions {
        if !held && exposed(offer, j) {
            exclusions.push(Exclusion::Jurisdiction { jurisdiction: j.clone(), policy: p.clone() });
        }
    }

    // 9. Key custody.
    if let Some(by) = customer_keys {
        if !held {
            exclusions.push(Exclusion::KeyCustody { required_by: by.to_string() });
        }
    }

    // 10. Latency: from every region the users are in, a measured p95
    // within the SLO. Unmeasured fails closed.
    if let Some(need_ms) = slo {
        for from in slo_from {
            let have_ms = offer.latency.iter().find(|(r, _)| r == from).map(|(_, ms)| *ms);
            if !matches!(have_ms, Some(h) if h <= need_ms) {
                exclusions.push(Exclusion::Latency { from: from.clone(), need_ms, have_ms });
            }
        }
    }

    OfferVerdict {
        offer_id: offer.id.clone(),
        vendor: offer.vendor.clone(),
        exclusions,
    }
}

/// Only the rules a resource's location decides — residency, policy
/// country, vendor and jurisdiction denials, and key custody — for checking
/// infrastructure that already says where it runs (`check-plan`). The same
/// evaluator, with the offer-only rules (class, trust, capacity, features)
/// left out.
pub fn check_location(offer: &Offer, residency: &Residency, customer_keys: Option<&str>, eff: &Effective) -> Vec<Exclusion> {
    let none = Resources::default();
    let need = Need { class: "", residency, trust: 0, resources: &none, features: &[], customer_keys, slo: None, slo_from: &[] };
    eval_offer(offer, &need, eff)
        .exclusions
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                Exclusion::Residency { .. }
                    | Exclusion::PolicyCountry { .. }
                    | Exclusion::PolicyVendor { .. }
                    | Exclusion::Jurisdiction { .. }
                    | Exclusion::KeyCustody { .. }
            )
        })
        .collect()
}

/// Whether the customer holds the offer's keys. Unstated means the vendor does.
pub fn customer_keys_held(offer: &Offer) -> bool {
    customer_keys(&offer.keys)
}

/// Whether jurisdiction `j`'s law reaches the offer's operator.
pub fn exposed(offer: &Offer, j: &str) -> bool {
    offer.country.as_deref() == Some(j) || offer.jurisdictions.iter().any(|x| x == j)
}

/// What requires customer-held keys for a unit: the unit itself, then a data
/// dependency, then a policy.
fn keys_required_by(
    kind: &str,
    name: &str,
    keys: &Option<String>,
    deps: &[&DataDecl],
    eff: &Effective,
) -> Option<String> {
    if customer_keys(keys) {
        return Some(format!("{} `{}`", kind, name));
    }
    if let Some(d) = deps.iter().find(|d| customer_keys(&d.keys)) {
        return Some(format!("data `{}`", d.name));
    }
    eff.customer_keys.as_ref().map(|p| format!("policy `{}`", p))
}

fn plan_service(w: &Workload, s: &Service, offers: &[Offer], eff: &Effective) -> UnitPlan {
    // Effective residency/sensitivity fold in each dependency, then policy.
    let mut residency = fold_residency(None, s.residency.as_deref());
    let mut sensitivity = s.sensitivity.unwrap_or(0);
    for dep in &s.depends_on {
        if let Some(d) = w.data.iter().find(|d| &d.name == dep) {
            residency = fold_residency(residency, d.residency.as_deref());
            if let Some(ds) = d.sensitivity {
                sensitivity = sensitivity.max(ds);
            }
        }
    }
    let residency = fold_residencies(residency, eff.residency.as_ref()).unwrap_or(Residency::Global);
    let need_trust = min_trust(sensitivity).max(eff.min_trust);
    let requires = s.required_features();
    let deps: Vec<&DataDecl> =
        s.depends_on.iter().filter_map(|dep| w.data.iter().find(|d| &d.name == dep)).collect();
    let customer_keys = keys_required_by("service", &s.name, &s.keys, &deps, eff);

    let verdicts = offers
        .iter()
        .map(|o| {
            let need = Need {
                class: &s.class,
                residency: &residency,
                trust: need_trust,
                resources: &s.resources,
                features: &requires,
                customer_keys: customer_keys.as_deref(),
                slo: s.slo_p95_ms,
                slo_from: &s.slo_from,
            };
            eval_offer(o, &need, eff)
        })
        .collect();

    UnitPlan {
        kind: "service",
        name: s.name.clone(),
        class: s.class.clone(),
        effective_residency: residency.display(),
        residency,
        effective_sensitivity: sensitivity,
        min_trust: need_trust,
        min_vendors: s.min_vendors,
        resources: s.resources.clone(),
        requires,
        customer_keys,
        slo_p95_ms: s.slo_p95_ms,
        slo_from: s.slo_from.clone(),
        verdicts,
    }
}

fn plan_job(j: &Job, offers: &[Offer], eff: &Effective) -> UnitPlan {
    let sensitivity = j.sensitivity.unwrap_or(0);
    let residency = fold_residencies(fold_residency(None, j.residency.as_deref()), eff.residency.as_ref())
        .unwrap_or(Residency::Global);
    let need_trust = min_trust(sensitivity).max(eff.min_trust);
    let customer_keys = keys_required_by("job", &j.name, &j.keys, &[], eff);
    let verdicts = offers
        .iter()
        .map(|o| {
            let need = Need {
                class: &j.class,
                residency: &residency,
                trust: need_trust,
                resources: &j.resources,
                features: &[],
                customer_keys: customer_keys.as_deref(),
                slo: None,
                slo_from: &[],
            };
            eval_offer(o, &need, eff)
        })
        .collect();
    UnitPlan {
        kind: "job",
        name: j.name.clone(),
        class: j.class.clone(),
        effective_residency: residency.display(),
        residency,
        effective_sensitivity: sensitivity,
        min_trust: need_trust,
        min_vendors: None,
        resources: j.resources.clone(),
        requires: Vec::new(),
        customer_keys,
        slo_p95_ms: None,
        slo_from: Vec::new(),
        verdicts,
    }
}

/// Compute feasibility for one workload against a catalog of offers, under
/// a set of policies (pass `&[]` for none).
pub fn plan_workload(w: &Workload, offers: &[Offer], policies: &[Policy]) -> WorkloadPlan {
    let eff = effective(policies);
    let mut units = Vec::new();
    for s in &w.services {
        units.push(plan_service(w, s, offers, &eff));
    }
    for j in &w.jobs {
        units.push(plan_job(j, offers, &eff));
    }
    WorkloadPlan { workload: w.name.clone(), policies: eff.names.clone(), units }
}

/// Compute feasibility for every workload in a file, under the file's
/// policies (including any merged in by the loader).
pub fn plan_file(file: &File, offers: &[Offer]) -> Vec<WorkloadPlan> {
    file.workloads
        .iter()
        .map(|w| plan_workload(w, offers, &file.policies))
        .collect()
}
