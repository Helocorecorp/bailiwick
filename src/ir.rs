//! The canonical intermediate representation (IR), version `bwk/0`.
//!
//! Tools exchange this, not the surface syntax: a Terraform provider, a
//! compose converter or the optimizer can read or emit it without a
//! parser. Durations are milliseconds, money is dollars, data sizes are
//! GB (2^30 bytes), levels are integers. Field order is fixed so the
//! output is byte-stable. The Protobuf schema for the same shape lives in
//! `spec/ir/bwk.proto`.

use crate::ast::*;
use crate::json::Json;

pub const VERSION: &str = "bwk/0";

fn budget(b: &Option<Budget>) -> Json {
    match b {
        None => Json::Null,
        Some(b) => Json::obj(vec![
            ("amount_usd", Json::Num(b.amount)),
            ("period", Json::str(&b.period)),
        ]),
    }
}

pub fn resources(r: &Resources) -> Json {
    if r.is_empty() {
        return Json::Null;
    }
    Json::obj(vec![
        ("vcpu", Json::opt_f64(r.vcpu)),
        ("mem_gb", Json::opt_f64(r.mem_gb)),
        ("disk_gb", Json::opt_f64(r.disk_gb)),
        ("gpu", Json::opt_f64(r.gpu)),
    ])
}

fn policy(p: &Policy) -> Json {
    Json::obj(vec![
        ("name", Json::str(&p.name)),
        ("residency", Json::opt_str(&p.residency)),
        ("min_trust", Json::opt_u8(p.min_trust)),
        ("deny_vendors", Json::strs(&p.deny_vendors)),
        ("deny_countries", Json::strs(&p.deny_countries)),
        ("deny_jurisdictions", Json::strs(&p.deny_jurisdictions)),
        ("keys", Json::opt_str(&p.keys)),
        ("budget_cap", budget(&p.budget_cap)),
    ])
}

fn data(d: &DataDecl) -> Json {
    Json::obj(vec![
        ("name", Json::str(&d.name)),
        ("sensitivity", Json::opt_u8(d.sensitivity)),
        ("residency", Json::opt_str(&d.residency)),
        ("keys", Json::opt_str(&d.keys)),
    ])
}

fn service(s: &Service) -> Json {
    Json::obj(vec![
        ("name", Json::str(&s.name)),
        ("class", Json::str(&s.class)),
        ("image", Json::opt_str(&s.image)),
        ("sensitivity", Json::opt_u8(s.sensitivity)),
        ("residency", Json::opt_str(&s.residency)),
        ("slo_p95_ms", Json::opt_f64(s.slo_p95_ms)),
        ("slo_from", Json::strs(&s.slo_from)),
        ("min_vendors", Json::opt_i64(s.min_vendors)),
        ("depends_on", Json::strs(&s.depends_on)),
        ("resources", resources(&s.resources)),
        ("value_latency_usd_per_ms_per_request", Json::opt_f64(s.value_latency)),
        (
            "cache",
            Json::Arr(
                s.cache
                    .iter()
                    .map(|c| {
                        Json::obj(vec![
                            ("path", Json::str(&c.path)),
                            ("ttl_ms", Json::opt_f64(c.ttl_ms)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("requires", Json::strs(&s.requires)),
    ])
}

fn job(j: &Job) -> Json {
    Json::obj(vec![
        ("name", Json::str(&j.name)),
        ("class", Json::str(&j.class)),
        ("image", Json::opt_str(&j.image)),
        ("deadline_ms", Json::opt_f64(j.deadline_ms)),
        ("checkpointable", Json::opt_bool(j.checkpointable)),
        ("sensitivity", Json::opt_u8(j.sensitivity)),
        ("residency", Json::opt_str(&j.residency)),
        ("resources", resources(&j.resources)),
    ])
}

fn workload(w: &Workload) -> Json {
    Json::obj(vec![
        ("name", Json::str(&w.name)),
        ("profile", Json::opt_str(&w.profile)),
        ("budget", budget(&w.budget)),
        ("value_latency_usd_per_ms_per_request", Json::opt_f64(w.value_latency)),
        ("data", Json::Arr(w.data.iter().map(data).collect())),
        ("services", Json::Arr(w.services.iter().map(service).collect())),
        ("jobs", Json::Arr(w.jobs.iter().map(job).collect())),
    ])
}

fn offer(o: &Offer) -> Json {
    let price = match &o.price {
        None => Json::Null,
        Some(p) => Json::obj(vec![
            ("amount_usd", Json::Num(p.amount)),
            ("unit", Json::str(&p.unit)),
        ]),
    };
    Json::obj(vec![
        ("id", Json::str(&o.id)),
        ("vendor", Json::opt_str(&o.vendor)),
        ("country", Json::opt_str(&o.country)),
        ("subdivision", Json::opt_str(&o.subdivision)),
        ("trust", Json::opt_u8(o.trust)),
        ("classes", Json::strs(&o.classes)),
        ("price", price),
        ("resources", resources(&o.resources)),
        ("features", Json::strs(&o.features)),
        ("jurisdictions", Json::strs(&o.jurisdictions)),
        (
            "latency_p95_ms",
            Json::Arr(o.latency.iter().map(|(r, ms)| Json::obj(vec![("from", Json::str(r)), ("p95_ms", Json::Num(*ms))])).collect()),
        ),
        ("keys", Json::opt_str(&o.keys)),
    ])
}

/// Build the IR for a (checked, import-resolved) file.
pub fn to_json(file: &File) -> Json {
    Json::obj(vec![
        ("version", Json::str(VERSION)),
        (
            "imports",
            Json::Arr(file.imports.iter().map(|i| Json::str(&i.path)).collect()),
        ),
        ("policies", Json::Arr(file.policies.iter().map(policy).collect())),
        ("workloads", Json::Arr(file.workloads.iter().map(workload).collect())),
        ("offers", Json::Arr(file.offers.iter().map(offer).collect())),
    ])
}

pub fn to_string(file: &File) -> String {
    crate::json::to_string_pretty(&to_json(file))
}
