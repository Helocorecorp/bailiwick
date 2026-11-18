//! Importers: turn existing deployment files into BWK skeletons so nobody
//! starts from a blank page. An importer can recover *structure* (which
//! services exist, what they depend on, what they are sized at, where they
//! happened to run) but not *intent* (residency as a requirement,
//! sensitivity, SLOs). The output marks every inferred constraint with a
//! TODO comment, and is always valid BWK that passes `bailiwick check`.

pub mod compose;
pub mod fly;
pub mod hcl_min;
pub mod k8s;
pub mod terraform;
pub mod toml_min;
pub mod yaml_min;

use crate::ast::Resources;

/// The result of an import: BWK source plus notes about what could not be
/// inferred and must be confirmed by a human.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub bwk: String,
    pub notes: Vec<String>,
}

/// Make an arbitrary name a valid BWK identifier: letters, digits,
/// underscores and internal hyphens, starting with a letter or underscore.
pub fn sanitize_ident(s: &str) -> String {
    let mut out = String::new();
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_hyphen = false;
        } else if c == '_' {
            // A hyphen must be followed by an alphanumeric, so drop an
            // underscore that would directly follow one.
            if !prev_hyphen {
                out.push('_');
            }
        } else if (c == '-' || c == '.' || c == ' ' || c == '/' || c == ':')
            && !out.is_empty()
            && !prev_hyphen
        {
            out.push('-');
            prev_hyphen = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return "app".to_string();
    }
    let first = out.chars().next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        out.insert(0, '_');
    }
    out
}

/// Print a number without a trailing `.0`, with at most four decimals.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// A CPU quantity: Kubernetes millicores (`500m`), or a plain number.
///
/// A literal too large for `f64` parses as infinity rather than failing, and
/// a quantity that is not finite is not a quantity: it would reach the
/// output as `vcpu >= inf`, which no longer checks. Such a value is read as
/// no value at all, the same as text that is not a number.
pub fn parse_cpu(s: &str) -> Option<f64> {
    let t = s.trim();
    let v = match t.strip_suffix('m') {
        Some(m) => m.trim().parse::<f64>().ok().map(|v| v / 1000.0),
        None => t.parse::<f64>().ok(),
    };
    v.filter(|v| v.is_finite())
}

/// A memory/disk quantity, normalized to BWK GB (2^30 bytes).
///
/// `Ki/Mi/Gi/Ti` and `kb/mb/gb/tb` are binary. Bare `k/m/g/t` are decimal
/// in Kubernetes (`500M` = 500,000,000 bytes) but binary in Docker and Fly
/// (`512M` = 512 MiB); `k8s_style` picks the convention. A value that is
/// not finite is read as no value, for the reason given on `parse_cpu`.
pub fn parse_mem_gb(s: &str, k8s_style: bool) -> Option<f64> {
    let t = s.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    if num.is_empty() {
        return None;
    }
    let v: f64 = num.parse().ok()?;
    let u = unit.trim().to_lowercase();
    let (binary, pow): (bool, i32) = match u.as_str() {
        "" | "b" => (true, 0),
        "ki" | "kib" | "kb" => (true, 1),
        "mi" | "mib" | "mb" => (true, 2),
        "gi" | "gib" | "gb" => (true, 3),
        "ti" | "tib" | "tb" => (true, 4),
        "k" => (!k8s_style, 1),
        "m" => (!k8s_style, 2),
        "g" => (!k8s_style, 3),
        "t" => (!k8s_style, 4),
        _ => return None,
    };
    let base: f64 = if binary { 1024.0 } else { 1000.0 };
    let bytes = v * base.powi(pow);
    let gb = bytes / 1024f64.powi(3);
    let rounded = (gb * 10000.0).round() / 10000.0;
    rounded.is_finite().then_some(rounded)
}

/// `resources { vcpu >= 2  mem >= 4 GB }` for a unit, or None when empty.
pub fn resources_line(r: &Resources) -> Option<String> {
    if r.is_empty() {
        return None;
    }
    let parts: Vec<String> = r
        .fields()
        .into_iter()
        .map(|(n, v, u)| {
            if u.is_empty() {
                format!("{} >= {}", n, fmt_num(v))
            } else {
                format!("{} >= {} {}", n, fmt_num(v), u)
            }
        })
        .collect();
    Some(format!("resources {{ {} }}", parts.join("  ")))
}

/// Fly.io region code → BWK residency region, when one exists.
pub fn fly_region_to_residency(region: &str) -> Option<&'static str> {
    match region {
        "ams" | "arn" | "cdg" | "fra" | "mad" | "otp" | "waw" => Some("eu"),
        "lhr" => Some("uk"),
        "atl" | "bos" | "den" | "dfw" | "ewr" | "iad" | "lax" | "mia" | "ord" | "phx" | "sea"
        | "sjc" => Some("us"),
        "yul" | "yyz" => Some("ca"),
        "bom" | "hkg" | "nrt" | "sin" | "syd" => Some("apac"),
        _ => None,
    }
}

/// A cloud provider's region/location code → BWK residency region.
pub fn cloud_region_to_residency(provider: &str, region: &str) -> Option<&'static str> {
    let r = region.trim().to_lowercase();
    let r3 = r.get(..3).unwrap_or("");
    match provider {
        "aws" => {
            if r.starts_with("eu-west-2") {
                Some("uk")
            } else if r.starts_with("eu-") {
                Some("eu")
            } else if r.starts_with("us-") {
                Some("us")
            } else if r.starts_with("ca-") {
                Some("ca")
            } else if r.starts_with("ap-") {
                Some("apac")
            } else {
                None
            }
        }
        "google" | "gcp" => {
            if r.starts_with("europe-west2") {
                Some("uk")
            } else if r.starts_with("europe-north") {
                // Finland (europe-north1) is in the EU; Sweden/Norway vary.
                Some("eu")
            } else if r.starts_with("europe-") {
                Some("eu")
            } else if r.starts_with("us-") {
                Some("us")
            } else if r.starts_with("northamerica-northeast") {
                Some("ca")
            } else if r.starts_with("asia-") || r.starts_with("australia-") {
                Some("apac")
            } else {
                None
            }
        }
        "digitalocean" => match r3 {
            "ams" | "fra" => Some("eu"),
            "nyc" | "sfo" => Some("us"),
            "tor" => Some("ca"),
            "lon" => Some("uk"),
            "sgp" | "blr" | "syd" => Some("apac"),
            _ => None,
        },
        "hcloud" | "hetzner" => match r3 {
            "fsn" | "nbg" | "hel" => Some("eu"),
            "ash" | "hil" => Some("us"),
            "sin" => Some("apac"),
            _ => None,
        },
        "azurerm" | "azure" => match r.as_str() {
            "westeurope" | "northeurope" | "germanywestcentral" | "germanynorth"
            | "francecentral" | "francesouth" | "swedencentral" | "polandcentral"
            | "italynorth" | "spaincentral" => Some("eu"),
            "norwayeast" | "norwaywest" => Some("eea"),
            "uksouth" | "ukwest" => Some("uk"),
            "canadacentral" | "canadaeast" => Some("ca"),
            "eastus" | "eastus2" | "westus" | "westus2" | "westus3" | "centralus"
            | "northcentralus" | "southcentralus" | "westcentralus" => Some("us"),
            "japaneast" | "japanwest" | "australiaeast" | "australiasoutheast"
            | "southeastasia" | "koreacentral" | "koreasouth" | "centralindia"
            | "southindia" | "westindia" => Some("apac"),
            _ => None,
        },
        "fly" => fly_region_to_residency(&r),
        _ => None,
    }
}

/// Try every provider's region scheme (for labels that don't say which
/// cloud they came from, such as Kubernetes topology labels).
pub fn any_cloud_region_to_residency(region: &str) -> Option<&'static str> {
    for p in ["aws", "google", "digitalocean", "hcloud", "azurerm", "fly"] {
        if let Some(r) = cloud_region_to_residency(p, region) {
            return Some(r);
        }
    }
    None
}

/// Classify a container image into an BWK asset class, and whether it is
/// a stateful store (which also yields a `data` block).
pub fn classify_image(service_name: &str, image: Option<&str>) -> (&'static str, bool) {
    let img = image.unwrap_or("").to_lowercase();
    let base = img
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .to_string();
    let n = service_name.to_lowercase();

    const STORES: [&str; 20] = [
        "postgres", "postgresql", "mysql", "mariadb", "mongo", "mongodb", "redis", "valkey",
        "memcached", "elasticsearch", "opensearch", "clickhouse", "cassandra", "minio",
        "rabbitmq", "kafka", "nats", "etcd", "cockroachdb", "timescaledb",
    ];
    if STORES.iter().any(|s| base == *s || base.starts_with(&format!("{}-", s)))
        || ["db", "database", "postgres", "redis", "cache", "mongo"].contains(&n.as_str())
    {
        return ("database", true);
    }
    if ["nginx", "httpd", "caddy"].iter().any(|s| base == *s || base.starts_with(&format!("{}-", s))) {
        return ("frontend-static", false);
    }
    if n.contains("worker")
        || n.contains("cron")
        || n.contains("scheduler")
        || n.contains("beat")
        || n.contains("consumer")
    {
        return ("background-jobs", false);
    }
    ("backend-api", false)
}

/// Seconds → the largest exact BWK duration literal (`6 h`, `90 m`, `45 s`).
pub fn duration_literal_from_seconds(secs: f64) -> String {
    if secs >= 3600.0 && (secs / 3600.0).fract() == 0.0 {
        format!("{} h", fmt_num(secs / 3600.0))
    } else if secs >= 60.0 && (secs / 60.0).fract() == 0.0 {
        format!("{} m", fmt_num(secs / 60.0))
    } else {
        format!("{} s", fmt_num(secs))
    }
}
