//! The AST for BWK. Values are stored already normalized to canonical
//! units (durations in milliseconds, money in dollars, data sizes in GB),
//! because the parser does unit checking as it reads each value.

/// A parsed file. Demand (workloads), supply (offers), governance
/// (policies) and imports may all appear in one file.
#[derive(Clone, Debug, Default)]
pub struct File {
    pub imports: Vec<Import>,
    pub policies: Vec<Policy>,
    pub workloads: Vec<Workload>,
    pub offers: Vec<Offer>,
}

/// `import "relative/path.bwk"` — pulls in another file's policies and
/// offers. Resolved by the loader relative to the importing file.
#[derive(Clone, Debug)]
pub struct Import {
    pub path: String,
    pub line: usize,
    pub col: usize,
}

/// An organization policy. Policies can only *narrow*: every field here
/// either intersects (residency), raises a floor (min_trust), removes
/// candidates (deny), or lowers a cap (budget). See `policy.rs`.
#[derive(Clone, Debug)]
pub struct Policy {
    pub name: String,
    pub residency: Option<String>,
    pub min_trust: Option<u8>,
    pub deny_vendors: Vec<String>,
    pub deny_countries: Vec<String>,
    /// Jurisdictions whose law must not be able to compel plaintext:
    /// an offer within their reach is excluded unless the customer holds
    /// the keys.
    pub deny_jurisdictions: Vec<String>,
    /// `keys = customer`: everything under the policy needs customer-held keys.
    pub keys: Option<String>,
    pub budget_cap: Option<Budget>,
    pub line: usize,
    pub col: usize,
}

/// Compute resources. On a service or job these are *minimums*
/// (`vcpu >= 2`); on an offer they are what it *provides* (`vcpu = 16`).
/// Data sizes are in GB (2^30 bytes); vcpu and gpu are counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resources {
    pub vcpu: Option<f64>,
    pub mem_gb: Option<f64>,
    pub disk_gb: Option<f64>,
    pub gpu: Option<f64>,
}

impl Resources {
    pub fn is_empty(&self) -> bool {
        self.vcpu.is_none() && self.mem_gb.is_none() && self.disk_gb.is_none() && self.gpu.is_none()
    }

    /// (name, required value, unit suffix) for each set field, in canonical order.
    pub fn fields(&self) -> Vec<(&'static str, f64, &'static str)> {
        let mut v = Vec::new();
        if let Some(x) = self.vcpu {
            v.push(("vcpu", x, ""));
        }
        if let Some(x) = self.mem_gb {
            v.push(("mem", x, "GB"));
        }
        if let Some(x) = self.disk_gb {
            v.push(("disk", x, "GB"));
        }
        if let Some(x) = self.gpu {
            v.push(("gpu", x, ""));
        }
        v
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        match name {
            "vcpu" => self.vcpu,
            "mem" => self.mem_gb,
            "disk" => self.disk_gb,
            "gpu" => self.gpu,
            _ => None,
        }
    }
}

/// An edge cache rule: `cache "/assets/*" for 30 d` or `cache "/api/*" never`.
/// A service with any cache rule implicitly requires the `cache` feature.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheRule {
    pub path: String,
    /// TTL in milliseconds; `None` means never cache.
    pub ttl_ms: Option<f64>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct Workload {
    pub name: String,
    pub profile: Option<String>,
    pub budget: Option<Budget>,
    /// Default value of latency for every service: dollars per millisecond
    /// per request. A preference consumed by the optimizer, never a filter.
    pub value_latency: Option<f64>,
    pub data: Vec<DataDecl>,
    pub services: Vec<Service>,
    pub jobs: Vec<Job>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// Dollars.
    pub amount: f64,
    /// Period unit, e.g. "month".
    pub period: String,
}

#[derive(Clone, Debug)]
pub struct DataDecl {
    pub name: String,
    pub sensitivity: Option<u8>,
    pub residency: Option<String>,
    /// `keys = customer`: anything depending on this data needs customer-held keys.
    pub keys: Option<String>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct Service {
    pub name: String,
    pub class: String,
    pub image: Option<String>,
    pub sensitivity: Option<u8>,
    pub residency: Option<String>,
    /// p95 SLO in milliseconds.
    pub slo_p95_ms: Option<f64>,
    /// Where the service's users are (`slo.p95 <= 200 ms from eu`). With
    /// any, the SLO is a placement rule: an offer must have a measured p95
    /// from each of them within the target. Without, the SLO is only a
    /// target for choosing among allowed offers.
    pub slo_from: Vec<String>,
    pub min_vendors: Option<i64>,
    pub depends_on: Vec<String>,
    pub resources: Resources,
    /// Dollars per millisecond per request; overrides the workload default.
    pub value_latency: Option<f64>,
    pub cache: Vec<CacheRule>,
    /// Edge features the service needs an offer to provide.
    pub requires: Vec<String>,
    /// `keys = customer`: the service needs customer-held keys.
    pub keys: Option<String>,
    pub line: usize,
    pub col: usize,
}

impl Service {
    /// Explicit `requires` plus the implicit `cache` feature when cache
    /// rules are present, in a stable order.
    pub fn required_features(&self) -> Vec<String> {
        let mut out = self.requires.clone();
        if !self.cache.is_empty() && !out.iter().any(|f| f == "cache") {
            out.push("cache".to_string());
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub name: String,
    pub class: String,
    pub image: Option<String>,
    /// Deadline in milliseconds.
    pub deadline_ms: Option<f64>,
    pub checkpointable: Option<bool>,
    pub sensitivity: Option<u8>,
    pub residency: Option<String>,
    pub resources: Resources,
    /// `keys = customer`: the job needs customer-held keys.
    pub keys: Option<String>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct Offer {
    pub id: String,
    pub vendor: Option<String>,
    pub country: Option<String>,
    /// ISO 3166-2 subdivision within `country` (e.g. `ca` for California),
    /// when the provider states it. Required to satisfy a sub-national
    /// residency.
    pub subdivision: Option<String>,
    pub trust: Option<u8>,
    pub classes: Vec<String>,
    pub price: Option<Price>,
    /// What the offer provides. An offer that declares nothing cannot
    /// satisfy a unit that requires anything.
    pub resources: Resources,
    /// Edge features the offer provides (`cache`, `waf`, ...).
    pub features: Vec<String>,
    /// Jurisdictions, besides the offer's own country, whose law reaches
    /// its operator (e.g. the vendor's home country).
    pub jurisdictions: Vec<String>,
    /// Who holds the encryption keys: `customer` or `vendor`. Unstated
    /// means `vendor`: custody is never assumed.
    pub keys: Option<String>,
    /// Measured p95 latency, in milliseconds, from users in each region
    /// (`latency { p95 from eu = 35 ms }`).
    pub latency: Vec<(String, f64)>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct Price {
    /// Dollars.
    pub amount: f64,
    /// Price unit, e.g. "vcpu_hour".
    pub unit: String,
}

/// Whether a `keys` value asks for customer-held keys.
pub fn customer_keys(keys: &Option<String>) -> bool {
    keys.as_deref() == Some("customer")
}
