//! Spec-fixed semantic tables: jurisdiction regions as *sets* of location
//! codes (so they may overlap and intersect exactly, down to sub-national
//! subdivisions), the sensitivity → minimum-trust floor, and the known
//! profiles, classes and edge features. These are *normative*: every
//! conforming implementation must agree on them, which is what makes a
//! residency claim portable across tools.
//!
//! Location codes are lowercase ISO 3166-1 alpha-2 countries (`de`), or a
//! country and an ISO 3166-2 subdivision joined by a hyphen (`us-ca`). A
//! country code covers all of its subdivisions.

use std::collections::BTreeSet;

/// Display form of an empty residency intersection (e.g. `eu ∩ us`). It is
/// not a user-writable region.
pub const CONFLICT: &str = "<conflict>";

const EU: &[&str] = &[
    "at", "be", "bg", "hr", "cy", "cz", "dk", "ee", "fi", "fr", "de", "gr", "hu", "ie", "it",
    "lv", "lt", "lu", "mt", "nl", "pl", "pt", "ro", "sk", "si", "es", "se",
];
const EEA: &[&str] = &[
    "at", "be", "bg", "hr", "cy", "cz", "dk", "ee", "fi", "fr", "de", "gr", "hu", "ie", "it",
    "lv", "lt", "lu", "mt", "nl", "pl", "pt", "ro", "sk", "si", "es", "se", "is", "li", "no",
];
const NORDICS: &[&str] = &["dk", "fi", "is", "no", "se"];
const US: &[&str] = &["us"];
const CA: &[&str] = &["ca"];
const UK: &[&str] = &["gb"];
const APAC: &[&str] = &["jp", "sg", "au", "kr", "in"];
const CALIFORNIA: &[&str] = &["us-ca"];
const QUEBEC: &[&str] = &["ca-qc"];
const BRAZIL: &[&str] = &["br"];
const CHINA: &[&str] = &["cn"];
const INDIA: &[&str] = &["in"];
const AUSTRALIA: &[&str] = &["au"];
/// The EEA plus the countries with an EU adequacy decision under GDPR
/// Art. 45. The United States is left out: its adequacy (the EU–US Data
/// Privacy Framework) covers only certified organisations, which a
/// location cannot show.
const GDPR_ADEQUATE: &[&str] = &[
    "at", "be", "bg", "hr", "cy", "cz", "dk", "ee", "fi", "fr", "de", "gr", "hu", "ie", "it",
    "lv", "lt", "lu", "mt", "nl", "pl", "pt", "ro", "sk", "si", "es", "se", "is", "li", "no",
    "ad", "ar", "ca", "fo", "gg", "il", "im", "jp", "je", "nz", "kr", "ch", "gb", "uy",
];
/// The UK plus the countries covered by UK adequacy regulations: the EEA,
/// Gibraltar, and the EU-adequate countries carried over. The United States
/// is left out for the same reason as in `gdpr-adequate`.
const UK_ADEQUATE: &[&str] = &[
    "gb", "at", "be", "bg", "hr", "cy", "cz", "dk", "ee", "fi", "fr", "de", "gr", "hu", "ie",
    "it", "lv", "lt", "lu", "mt", "nl", "pl", "pt", "ro", "sk", "si", "es", "se", "is", "li",
    "no", "gi", "ad", "ar", "ca", "fo", "gg", "il", "im", "jp", "je", "nz", "kr", "ch", "uy",
];

/// Named regions, in display-preference order: when an intersection
/// equals a named set, the first matching name is used.
pub const REGIONS: &[(&str, &[&str])] = &[
    ("eu", EU),
    ("eea", EEA),
    ("nordics", NORDICS),
    ("us", US),
    ("ca", CA),
    ("uk", UK),
    ("apac", APAC),
    ("california", CALIFORNIA),
    ("quebec", QUEBEC),
    ("brazil", BRAZIL),
    ("china", CHINA),
    ("india", INDIA),
    ("australia", AUSTRALIA),
    ("gdpr-adequate", GDPR_ADEQUATE),
    ("uk-adequate", UK_ADEQUATE),
];

/// Whether a residency region string is one the spec defines.
pub fn region_known(res: &str) -> bool {
    res == "global" || REGIONS.iter().any(|(n, _)| *n == res)
}

/// Comma-separated list of the valid region names, for messages.
pub fn region_names() -> String {
    let mut v = vec!["global"];
    v.extend(REGIONS.iter().map(|(n, _)| *n));
    v.join(", ")
}

/// The country part of a location code (`us-ca` → `us`).
pub fn country_of(code: &str) -> &str {
    code.split('-').next().unwrap_or(code)
}

/// Whether a set covers a location code: directly, or via its country.
fn covers(set: &BTreeSet<&'static str>, code: &str) -> bool {
    set.contains(code) || (code.contains('-') && set.contains(country_of(code)))
}

/// Drop any subdivision whose country is also present (the country already
/// covers it), so sets have one canonical form.
fn normalize(set: &mut BTreeSet<&'static str>) {
    let countries: Vec<&'static str> =
        set.iter().filter(|c| !c.contains('-')).copied().collect();
    set.retain(|c| !(c.contains('-') && countries.contains(&country_of(c))));
}

/// A residency constraint: unconstrained (`Global`) or an explicit set of
/// location codes. Intersection is set intersection with country →
/// subdivision coverage, so overlapping regions (`eea ⊃ eu`,
/// `nordics ∩ eu = {dk, fi, se}`, `us ∩ california = california`) compose
/// exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Residency {
    Global,
    Set(BTreeSet<&'static str>),
}

impl Residency {
    /// Look up a named region. `None` if the name is unknown.
    pub fn from_name(name: &str) -> Option<Residency> {
        if name == "global" {
            return Some(Residency::Global);
        }
        REGIONS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, set)| Residency::Set(set.iter().copied().collect()))
    }

    /// Like `from_name`, but an unknown name yields the empty set (nothing
    /// is feasible). `check` reports unknown names separately.
    pub fn from_name_lossy(name: &str) -> Residency {
        Self::from_name(name).unwrap_or_else(|| Residency::Set(BTreeSet::new()))
    }

    pub fn intersect(&self, other: &Residency) -> Residency {
        match (self, other) {
            (Residency::Global, x) | (x, Residency::Global) => x.clone(),
            (Residency::Set(a), Residency::Set(b)) => {
                let mut out = BTreeSet::new();
                for x in a {
                    if covers(b, x) {
                        out.insert(*x);
                    }
                }
                for y in b {
                    if covers(a, y) {
                        out.insert(*y);
                    }
                }
                normalize(&mut out);
                Residency::Set(out)
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Residency::Set(s) if s.is_empty())
    }

    /// Whether an offer located in `country` (and, if known, `subdivision`)
    /// is inside this residency. A sub-national residency never admits an
    /// offer whose subdivision is unknown.
    pub fn contains(&self, country: &str, subdivision: Option<&str>) -> bool {
        match self {
            Residency::Global => true,
            Residency::Set(s) => {
                if s.contains(country) {
                    return true;
                }
                if let Some(sub) = subdivision {
                    let code = format!("{}-{}", country, sub);
                    return s.contains(code.as_str());
                }
                false
            }
        }
    }

    /// Whether this residency distinguishes sub-national locations.
    pub fn is_subnational(&self) -> bool {
        matches!(self, Residency::Set(s) if s.iter().any(|c| c.contains('-')))
    }

    /// Whether this residency names a subdivision of `country`.
    pub fn has_subdivisions_of(&self, country: &str) -> bool {
        matches!(self, Residency::Set(s) if s.iter().any(|c| c.contains('-') && country_of(c) == country))
    }

    /// `global`, a region name when the set equals one, `{a,b}` otherwise,
    /// or `<conflict>` when empty.
    pub fn display(&self) -> String {
        match self {
            Residency::Global => "global".to_string(),
            Residency::Set(s) => {
                if s.is_empty() {
                    return CONFLICT.to_string();
                }
                for (name, set) in REGIONS {
                    if set.len() == s.len() && set.iter().all(|c| s.contains(c)) {
                        return name.to_string();
                    }
                }
                format!("{{{}}}", s.iter().copied().collect::<Vec<_>>().join(","))
            }
        }
    }
}

/// Fold an optional region name into an accumulated constraint.
/// `None` means "unconstrained so far". Unknown names are skipped (they
/// are reported by `check`), so a typo cannot silently empty the set.
pub fn fold_residency(acc: Option<Residency>, name: Option<&str>) -> Option<Residency> {
    match name {
        None => acc,
        Some(n) => match Residency::from_name(n) {
            None => acc,
            Some(r) => Some(match acc {
                None => r,
                Some(a) => a.intersect(&r),
            }),
        },
    }
}

/// Fold two optional constraints.
pub fn fold_residencies(a: Option<Residency>, b: Option<&Residency>) -> Option<Residency> {
    match (a, b) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b.clone()),
        (Some(a), Some(b)) => Some(a.intersect(b)),
    }
}

/// Display of the intersection of two optional region names.
pub fn intersect_regions(a: Option<&str>, b: Option<&str>) -> Option<String> {
    fold_residency(fold_residency(None, a), b).map(|r| r.display())
}

/// The minimum trust level a workload of the given sensitivity may run on:
///   S0 → T1, S1 → T2, S2 → T2, S3 → T3.
pub fn min_trust(sensitivity: u8) -> u8 {
    match sensitivity {
        0 => 1,
        1 => 2,
        2 => 2,
        _ => 3,
    }
}

/// Profiles defined by the spec. Unknown profiles are a check-time error.
pub fn profile_known(p: &str) -> bool {
    PROFILES.contains(&p)
}

pub const PROFILES: &[&str] = &["cheapest", "balanced", "fastest", "sovereign", "green"];

/// The asset classes the spec recognizes.
pub fn class_known(c: &str) -> bool {
    CLASSES.contains(&c)
}

pub const CLASSES: &[&str] = &[
    "frontend-static",
    "backend-api",
    "media-assets",
    "database",
    "background-jobs",
    "ml-inference",
];

/// Edge features an offer may provide and a service may require. A
/// service with `cache` rules implicitly requires `cache`. Unknown names
/// are a check-time warning, not an error, so the list can grow.
pub const FEATURES: &[&str] = &[
    "cache",
    "waf",
    "websockets",
    "http3",
    "image-resizing",
    "edge-functions",
    // The offer runs OCI containers (a service with an `image`).
    "containers",
];

pub fn feature_known(f: &str) -> bool {
    FEATURES.contains(&f)
}
