//! Deployment certificates.
//!
//! `bailiwick certify` writes, for every unit of every workload, the exact
//! inputs the decision depended on (in the form the proven Lean model
//! reads), every offer's verdict, whether the unit deploys and, if one was
//! chosen, where it is placed. The certificate is bound to its inputs by a
//! SHA-256 digest of their canonical IR.
//!
//! An auditor checks a certificate offline, in two independent ways:
//!
//! - `bwk-model verify` (built from `formal/`) recomputes every verdict with
//!   the definitions the Lean theorems are about. A certificate it accepts
//!   inherits those theorems: every placement satisfies every policy, no
//!   denied jurisdiction can compel plaintext from it, and so on.
//! - `bailiwick verify` re-derives the certificate from the source files and
//!   requires it to be identical, so the case really is the program.

use crate::ast::{customer_keys, DataDecl, File, Job, Offer, Policy, Resources, Service, Workload};
use crate::feasibility::{plan_workload, Exclusion};
use crate::json::Json;
use crate::semantics::Residency;

pub const FORMAT: &str = "bailiwick-certificate/1";

/// A residency name as the model reads it: a list of locations, or `null`
/// for unconstrained (`global`, absent, or an unknown name, which the
/// evaluator skips and `check` reports).
fn residency(name: Option<&str>) -> Json {
    match name.and_then(Residency::from_name) {
        Some(Residency::Set(s)) => Json::strs(&s.into_iter().collect::<Vec<_>>()),
        _ => Json::Null,
    }
}

fn opt_num(v: Option<u8>) -> Json {
    v.map_or(Json::Null, |n| Json::Num(n as f64))
}

fn needs(r: &Resources) -> Json {
    Json::Arr(r.fields().into_iter().map(|(n, v, _)| Json::Arr(vec![Json::str(n), Json::Num(v)])).collect())
}

/// An offer as the model reads it.
pub fn offer_case(o: &Offer) -> Json {
    let res = ["vcpu", "mem", "disk", "gpu"]
        .iter()
        .filter_map(|n| o.resources.get(n).map(|v| Json::Arr(vec![Json::str(*n), Json::Num(v)])))
        .collect();
    Json::obj(vec![
        ("id", Json::str(&o.id)),
        ("vendor", Json::opt_str(&o.vendor)),
        ("country", Json::opt_str(&o.country)),
        ("sub", Json::opt_str(&o.subdivision)),
        ("trust", opt_num(o.trust)),
        ("classes", Json::strs(&o.classes)),
        ("resources", Json::Arr(res)),
        ("features", Json::strs(&o.features)),
        ("jurisdictions", Json::strs(&o.jurisdictions)),
        ("customer_keys", Json::Bool(customer_keys(&o.keys))),
        ("latency", Json::Arr(o.latency.iter().map(|(r, ms)| Json::Arr(vec![Json::str(r), Json::Num(*ms)])).collect())),
    ])
}

/// A policy as the model reads it. Budgets never decide feasibility.
pub fn policy_case(p: &Policy) -> Json {
    Json::obj(vec![
        ("name", Json::str(&p.name)),
        ("residency", residency(p.residency.as_deref())),
        ("min_trust", opt_num(p.min_trust)),
        ("deny_vendors", Json::strs(&p.deny_vendors)),
        ("deny_countries", Json::strs(&p.deny_countries)),
        ("deny_jurisdictions", Json::strs(&p.deny_jurisdictions)),
        ("customer_keys", Json::Bool(customer_keys(&p.keys))),
    ])
}

fn dep_case(d: &DataDecl) -> Json {
    Json::obj(vec![
        ("residency", residency(d.residency.as_deref())),
        ("sensitivity", opt_num(d.sensitivity)),
        ("customer_keys", Json::Bool(customer_keys(&d.keys))),
    ])
}

/// A routable unit of a workload.
#[derive(Clone, Copy)]
pub enum Unit<'a> {
    Service(&'a Service),
    Job(&'a Job),
}

/// The units of a workload, in plan order (services, then jobs).
pub fn units(w: &Workload) -> Vec<Unit<'_>> {
    w.services.iter().map(Unit::Service).chain(w.jobs.iter().map(Unit::Job)).collect()
}

/// A unit as the model reads it.
pub fn unit_case(w: &Workload, u: Unit) -> Json {
    match u {
        Unit::Service(s) => {
            let deps = s
                .depends_on
                .iter()
                .filter_map(|d| w.data.iter().find(|x| &x.name == d))
                .map(dep_case)
                .collect();
            Json::obj(vec![
                ("class", Json::str(&s.class)),
                ("residency", residency(s.residency.as_deref())),
                ("sensitivity", opt_num(s.sensitivity)),
                ("deps", Json::Arr(deps)),
                ("needs", needs(&s.resources)),
                ("features", Json::strs(&s.required_features())),
                ("min_vendors", s.min_vendors.map_or(Json::Null, |n| Json::Num(n as f64))),
                ("customer_keys", Json::Bool(customer_keys(&s.keys))),
                // Only an SLO with `from` regions is a placement rule.
                ("slo", if s.slo_from.is_empty() { Json::Null } else { s.slo_p95_ms.map_or(Json::Null, Json::Num) }),
                ("slo_from", Json::strs(&s.slo_from)),
            ])
        }
        Unit::Job(j) => Json::obj(vec![
            ("class", Json::str(&j.class)),
            ("residency", residency(j.residency.as_deref())),
            ("sensitivity", opt_num(j.sensitivity)),
            ("deps", Json::Arr(vec![])),
            ("needs", needs(&j.resources)),
            ("features", Json::Arr(vec![])),
            ("min_vendors", Json::Null),
            ("customer_keys", Json::Bool(customer_keys(&j.keys))),
            ("slo", Json::Null),
            ("slo_from", Json::Arr(vec![])),
        ]),
    }
}

/// The full case for one unit: the policies in force, the unit, the offers.
pub fn case(w: &Workload, u: Unit, offers: &[Offer], policies: &[Policy]) -> Json {
    Json::obj(vec![
        ("policies", Json::Arr(policies.iter().map(policy_case).collect())),
        ("unit", unit_case(w, u)),
        ("offers", Json::Arr(offers.iter().map(offer_case).collect())),
    ])
}

/// An exclusion as the model names it: its kind and identifying detail.
pub fn render(e: &Exclusion) -> String {
    match e {
        Exclusion::Capability { .. } => "capability".into(),
        Exclusion::Residency { .. } => "residency".into(),
        Exclusion::Trust { have, need } => format!("trust:{}<{}", have, need),
        Exclusion::PolicyVendor { policy, .. } => format!("policy_vendor:{}", policy),
        Exclusion::PolicyCountry { policy, .. } => format!("policy_country:{}", policy),
        Exclusion::Resource { resource, .. } => format!("resource:{}", resource),
        Exclusion::Feature { feature } => format!("feature:{}", feature),
        Exclusion::Jurisdiction { jurisdiction, policy } => format!("jurisdiction:{}:{}", jurisdiction, policy),
        Exclusion::KeyCustody { .. } => "key_custody".into(),
        Exclusion::Latency { from, .. } => format!("latency:{}", from),
        Exclusion::MissingField { field } => format!("missing_field:{}", field),
    }
}

/// A chosen placement: `unit` (a service or job name, or `workload.unit`)
/// runs on `offers`.
#[derive(Clone, Debug)]
pub struct Placement {
    pub unit: String,
    pub offers: Vec<String>,
}

impl Placement {
    /// Parse `unit=o1,o2`.
    pub fn parse(s: &str) -> Result<Placement, String> {
        let (unit, offers) = s.split_once('=').ok_or_else(|| format!("expected `unit=offer[,offer...]`, got `{}`", s))?;
        let offers: Vec<String> = offers.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        if unit.trim().is_empty() || offers.is_empty() {
            return Err(format!("expected `unit=offer[,offer...]`, got `{}`", s));
        }
        Ok(Placement { unit: unit.trim().to_string(), offers })
    }

    fn matches(&self, workload: &str, unit: &str) -> bool {
        self.unit == unit || self.unit == format!("{}.{}", workload, unit)
    }
}

/// Build the certificate for a loaded file evaluated against `offers`.
/// Fails if a placement names an unknown unit or offer, or places a unit
/// on an offer that is not feasible for it: a certificate is never written
/// for a placement that breaks a rule.
pub fn certify(file: &File, offers: &[Offer], placements: &[Placement]) -> Result<Json, String> {
    let mut used = vec![false; placements.len()];
    let mut out_units = Vec::new();
    let mut deployable = true;
    for w in &file.workloads {
        let plan = plan_workload(w, offers, &file.policies);
        for (u, up) in units(w).into_iter().zip(&plan.units) {
            let deploys = up.fails_closed().is_none();
            deployable &= deploys;
            let feasible: Vec<&str> = up.feasible_ids();
            let mut fields = vec![
                ("workload", Json::str(&w.name)),
                ("kind", Json::str(up.kind)),
                ("name", Json::str(&up.name)),
                ("case", case(w, u, offers, &file.policies)),
                (
                    "verdicts",
                    Json::Arr(
                        up.verdicts
                            .iter()
                            .map(|v| Json::Arr(v.exclusions.iter().map(|e| Json::str(render(e))).collect()))
                            .collect(),
                    ),
                ),
                ("deploys", Json::Bool(deploys)),
                ("feasible", Json::strs(&feasible)),
            ];
            if let Some(i) = placements.iter().position(|p| p.matches(&w.name, &up.name)) {
                used[i] = true;
                let p = &placements[i];
                for id in &p.offers {
                    let Some(v) = up.verdicts.iter().find(|v| &v.offer_id == id) else {
                        return Err(format!("placement for `{}`: no offer `{}`", p.unit, id));
                    };
                    if !v.feasible() {
                        let why: Vec<String> = v.exclusions.iter().map(|e| e.reason()).collect();
                        return Err(format!(
                            "refusing to certify: {} `{}` placed on `{}`, which is excluded: {}",
                            up.kind,
                            up.name,
                            id,
                            why.join("; ")
                        ));
                    }
                }
                fields.push(("placement", Json::strs(&p.offers)));
            }
            out_units.push(Json::obj(fields));
        }
    }
    if let Some(i) = used.iter().position(|u| !u) {
        return Err(format!("placement for `{}`: no such service or job", placements[i].unit));
    }

    let mut inputs = file.clone();
    inputs.offers = offers.to_vec();
    let digest = sha256_hex(crate::json::to_string_compact(&crate::ir::to_json(&inputs)).as_bytes());
    Ok(Json::obj(vec![
        ("format", Json::str(FORMAT)),
        ("tool", Json::str(concat!("bailiwick ", env!("CARGO_PKG_VERSION")))),
        ("inputs_sha256", Json::str(digest)),
        ("deployable", Json::Bool(deployable)),
        ("units", Json::Arr(out_units)),
    ]))
}

/// Length of the certificate's input digest (64 hex digits when well formed).
pub fn inputs_digest_len(cert: &Json) -> usize {
    cert.get("inputs_sha256").and_then(|d| d.as_str()).map_or(0, |d| d.len())
}

/// The placements recorded in a certificate.
pub fn placements_of(cert: &Json) -> Vec<Placement> {
    let mut out = Vec::new();
    for u in cert.get("units").and_then(|u| u.as_arr()).unwrap_or(&[]) {
        if let Some(p) = u.get("placement").and_then(|p| p.as_arr()) {
            let w = u.get("workload").and_then(|x| x.as_str()).unwrap_or("");
            let n = u.get("name").and_then(|x| x.as_str()).unwrap_or("");
            out.push(Placement {
                unit: format!("{}.{}", w, n),
                offers: p.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            });
        }
    }
    out
}

/// Check a certificate against the source it claims to describe: re-derive
/// it, with the same placements, and require it to be identical.
pub fn verify(cert: &Json, file: &File, offers: &[Offer]) -> Result<(), String> {
    if cert.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(format!("not a `{}` certificate", FORMAT));
    }
    let again = certify(file, offers, &placements_of(cert))?;
    if again.get("inputs_sha256") != cert.get("inputs_sha256") {
        return Err("the inputs differ from the ones the certificate was issued for".into());
    }
    // The tool version may differ; everything else must match exactly.
    let strip = |j: &Json| match j {
        Json::Obj(fs) => Json::Obj(fs.iter().filter(|(k, _)| k != "tool").cloned().collect()),
        other => other.clone(),
    };
    if crate::json::to_string_compact(&strip(&again)) != crate::json::to_string_compact(&strip(cert)) {
        return Err("the certificate does not match what these inputs produce".into());
    }
    Ok(())
}

// --- SHA-256 (FIPS 180-4), in-tree so the toolchain stays dependency-free ---

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_known_answers() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let a = vec![b'a'; 1_000_000];
        assert_eq!(sha256_hex(&a), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }
}
