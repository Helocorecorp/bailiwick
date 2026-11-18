//! Terraform (HCL) → BWK. Terraform describes *where and how* resources
//! are created; BWK describes *what each part requires*. The importer
//! recovers compute resources (services), databases and buckets (data
//! blocks), the dependencies between them (explicit `depends_on` and
//! references inside expressions), and the residency implied by each
//! resource's region or its provider's default — flagged for confirmation,
//! because "it was created in eu-central-1" is not "it must stay in the EU".

use super::hcl_min::{self, Body, Value};
use super::{cloud_region_to_residency, fmt_num, sanitize_ident, Imported};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Compute,
    Store,
    Objects,
    Static,
    Ignore,
}

fn classify(ty: &str) -> (Kind, &'static str) {
    match ty {
        "aws_instance"
        | "aws_ecs_service"
        | "aws_lambda_function"
        | "aws_apprunner_service"
        | "google_compute_instance"
        | "google_cloud_run_service"
        | "google_cloud_run_v2_service"
        | "digitalocean_droplet"
        | "digitalocean_app"
        | "hcloud_server"
        | "azurerm_linux_virtual_machine"
        | "azurerm_windows_virtual_machine"
        | "azurerm_container_app"
        | "fly_app"
        | "fly_machine" => (Kind::Compute, "backend-api"),
        "aws_db_instance"
        | "aws_rds_cluster"
        | "aws_elasticache_cluster"
        | "aws_elasticache_replication_group"
        | "aws_dynamodb_table"
        | "google_sql_database_instance"
        | "google_redis_instance"
        | "digitalocean_database_cluster"
        | "azurerm_postgresql_flexible_server"
        | "azurerm_mysql_flexible_server"
        | "azurerm_cosmosdb_account" => (Kind::Store, "database"),
        "aws_s3_bucket"
        | "aws_ebs_volume"
        | "google_storage_bucket"
        | "digitalocean_spaces_bucket"
        | "digitalocean_volume"
        | "azurerm_storage_account"
        | "cloudflare_r2_bucket"
        | "hcloud_volume" => (Kind::Objects, "database"),
        "aws_cloudfront_distribution" | "cloudflare_pages_project" | "vercel_project"
        | "netlify_site" => (Kind::Static, "frontend-static"),
        _ => (Kind::Ignore, ""),
    }
}

fn provider_of(ty: &str) -> &str {
    ty.split('_').next().unwrap_or("")
}

/// Reduce a zone/AZ to its region for lookup: `eu-central-1a` →
/// `eu-central-1`, `europe-west3-a` → `europe-west3`, `West Europe` →
/// `westeurope`.
fn normalize_region(provider: &str, raw: &str) -> String {
    let r = raw.trim().to_lowercase();
    match provider {
        "aws" => {
            let b = r.as_bytes();
            if b.len() > 2 && b[b.len() - 1].is_ascii_lowercase() && b[b.len() - 2].is_ascii_digit() {
                r[..r.len() - 1].to_string()
            } else {
                r
            }
        }
        "google" => {
            if let Some(idx) = r.rfind('-') {
                let tail = &r[idx + 1..];
                if tail.len() == 1 && tail.chars().all(|c| c.is_ascii_alphabetic()) {
                    return r[..idx].to_string();
                }
            }
            r
        }
        "azurerm" => r.replace(' ', ""),
        _ => r,
    }
}

/// The first region-like attribute: its text and whether it is a literal.
fn region_of(body: &Body) -> Option<(String, bool)> {
    for key in ["region", "location", "zone", "availability_zone"] {
        if let Some(v) = body.attr(key) {
            return match v {
                Value::Str(s) => Some((s.clone(), true)),
                Value::Raw(r) => Some((r.clone(), false)),
                _ => None,
            };
        }
    }
    None
}

/// A sizing attribute worth surfacing as a comment.
fn size_note(body: &Body) -> Option<String> {
    for key in ["instance_type", "machine_type", "size", "server_type", "instance_class"] {
        if let Some(v) = body.attr(key) {
            if let Some(t) = v.text() {
                return Some(format!("{} = {}", key, t));
            }
        }
    }
    None
}

/// How many copies `key` asks for, in words the reader can check against
/// their own file. A `count` or `for_each` driven by an expression cannot
/// be resolved without evaluating Terraform, so this says that rather than
/// guessing a number or staying quiet.
fn multiplicity_of(key: &str, v: &Value) -> String {
    match v {
        Value::Num(n) => format!("{} = {}", key, fmt_num(*n)),
        Value::List(l) => format!("{} over {} entries", key, l.len()),
        Value::Obj(o) => format!("{} over {} entries", key, o.len()),
        other => match other.text() {
            Some(t) => format!("{} = {}, which this importer does not evaluate", key, t),
            None => format!("{} is set to an expression this importer does not evaluate", key),
        },
    }
}

/// `count` or `for_each` on a resource: Terraform makes several copies of
/// it and this importer writes one unit, which understates the estate by
/// exactly that factor. So say what was seen, say it was not applied, and
/// say what to do about it — in the output, where the reader will meet it,
/// the way the Kubernetes importer treats `replicas`. Multiplying the
/// sizing instead would be worse: it would invent a number.
fn multiplicity_note(body: &Body) -> Option<String> {
    let seen = body
        .attr("count")
        .map(|v| multiplicity_of("count", v))
        .or_else(|| body.attr("for_each").map(|v| multiplicity_of("for_each", v)))?;
    Some(format!("{} — what follows describes one of them, not the whole set.", seen))
}

/// Write a `count`/`for_each` note inside a block. Three short comment
/// lines read better in the output than one long one.
fn push_multiplicity(out: &mut String, note: &str) {
    out.push_str(&format!("    // {}\n", note));
    out.push_str("    // Copies are not vendors: add `redundancy { vendors >= n }` if the\n");
    out.push_str("    // spread matters, and raise `resources` if the capacity does.\n");
}

/// Characters that continue a Terraform identifier.
fn is_addr_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Does `text` reference the resource address `addr` itself, rather than
/// merely begin with it? `aws_db_instance.main` must not match inside
/// `aws_db_instance.main_replica`, which is a different resource: counting
/// it would give a service a data dependency it does not have. A following
/// `.` is part of the reference (`aws_db_instance.main.address`).
fn mentions(text: &str, addr: &str) -> bool {
    if addr.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(i) = text[from..].find(addr) {
        let start = from + i;
        let end = start + addr.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_none_or(|c| !is_addr_char(c) && c != '.')
            && after.is_none_or(|c| !is_addr_char(c))
        {
            return true;
        }
        from = end;
    }
    false
}

struct Res {
    ty: String,
    name: String,
    kind: Kind,
    class: &'static str,
    ident: String,
    data_ident: Option<String>,
    residency: Option<(&'static str, String)>,
    leaves: Vec<String>,
    size: Option<String>,
    multiplicity: Option<String>,
    notes: Vec<String>,
}

pub fn import(src: &str) -> Result<Imported, String> {
    let root = hcl_min::parse(src)?;
    let mut notes = Vec::new();

    // Provider default regions.
    let mut provider_region: Vec<(String, Option<(&'static str, String)>)> = Vec::new();
    for b in root.blocks_of("provider") {
        let pname = b.labels.first().cloned().unwrap_or_default();
        let res = match region_of(&b.body) {
            Some((raw, true)) => {
                let r = cloud_region_to_residency(&pname, &normalize_region(&pname, &raw));
                if r.is_none() {
                    notes.push(format!(
                        "provider `{}` region `{}` has no BWK residency region",
                        pname, raw
                    ));
                }
                r.map(|x| (x, raw))
            }
            Some((raw, false)) => {
                notes.push(format!(
                    "provider `{}` region is an expression (`{}`); residency must be set manually",
                    pname, raw
                ));
                None
            }
            None => None,
        };
        provider_region.push((pname, res));
    }

    // Resources.
    let mut resources: Vec<Res> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    for b in root.blocks_of("resource") {
        if b.labels.len() < 2 {
            continue;
        }
        let ty = b.labels[0].clone();
        let name = b.labels[1].clone();
        let (kind, class) = classify(&ty);
        if kind == Kind::Ignore {
            ignored.push(format!("{}.{}", ty, name));
            continue;
        }
        let provider = provider_of(&ty).to_string();
        let mut rnotes = Vec::new();
        let residency = match region_of(&b.body) {
            Some((raw, true)) => {
                let r = cloud_region_to_residency(&provider, &normalize_region(&provider, &raw));
                if r.is_none() {
                    rnotes.push(format!("region `{}` has no BWK residency region", raw));
                }
                r.map(|x| (x, raw))
            }
            Some((raw, false)) => {
                rnotes.push(format!("residency depends on `{}`; set it manually", raw));
                None
            }
            // A resource with no region of its own inherits the provider's.
            // The evidence has to say so: a bare `from eu-central-1` sends
            // the reader looking for a string this resource does not carry.
            None => provider_region
                .iter()
                .find(|(p, _)| *p == provider)
                .and_then(|(_, r)| r.clone())
                .map(|(res, raw)| (res, format!("provider \"{}\" default {}", provider, raw))),
        };
        let mut leaves: Vec<&str> = Vec::new();
        b.body.leaves(&mut leaves);
        let leaves: Vec<String> = leaves.into_iter().map(|s| s.to_string()).collect();
        resources.push(Res {
            ident: sanitize_ident(&name),
            data_ident: None,
            ty,
            name,
            kind,
            class,
            residency,
            leaves,
            size: size_note(&b.body),
            multiplicity: multiplicity_note(&b.body),
            notes: rnotes,
        });
    }
    if resources.is_empty() {
        return Err(
            "no supported resources found (compute instances, databases, buckets, static sites)"
                .to_string(),
        );
    }

    // Unique identifiers: services first, then data blocks avoiding clashes.
    let mut taken: Vec<String> = Vec::new();
    for r in resources.iter_mut() {
        if matches!(r.kind, Kind::Compute | Kind::Store | Kind::Static) {
            let mut id = r.ident.clone();
            let mut n = 2;
            while taken.contains(&id) {
                id = format!("{}-{}", r.ident, n);
                n += 1;
            }
            r.ident = id.clone();
            taken.push(id);
        }
    }
    for r in resources.iter_mut() {
        match r.kind {
            Kind::Store => {
                let mut id = format!("{}-store", r.ident);
                let mut n = 2;
                while taken.contains(&id) {
                    id = format!("{}-store-{}", r.ident, n);
                    n += 1;
                }
                taken.push(id.clone());
                r.data_ident = Some(id);
            }
            Kind::Objects => {
                let base = sanitize_ident(&r.name);
                let mut id = base.clone();
                let mut n = 2;
                while taken.contains(&id) {
                    id = if n == 2 { format!("{}-bucket", base) } else { format!("{}-bucket-{}", base, n) };
                    n += 1;
                }
                taken.push(id.clone());
                r.data_ident = Some(id);
            }
            _ => {}
        }
    }

    // Dependencies: a unit depends on every store/bucket it references.
    let data_refs: Vec<(String, String)> = resources
        .iter()
        .filter_map(|r| r.data_ident.as_ref().map(|d| (format!("{}.{}", r.ty, r.name), d.clone())))
        .collect();
    let deps_of = |r: &Res| -> Vec<String> {
        let mut out = Vec::new();
        for (reference, data_ident) in &data_refs {
            if r.leaves.iter().any(|l| mentions(l, reference)) && !out.contains(data_ident) {
                out.push(data_ident.clone());
            }
        }
        out
    };

    for r in &resources {
        for n in &r.notes {
            notes.push(format!("{}.{}: {}", r.ty, r.name, n));
        }
    }
    if !ignored.is_empty() {
        notes.push(format!(
            "ignored {} resource(s) with no BWK equivalent: {}",
            ignored.len(),
            ignored.join(", ")
        ));
    }

    let mut out = String::new();
    out.push_str("// Imported from Terraform by `bailiwick import terraform`.\n");
    out.push_str("// Terraform says *where and how* resources were created; BWK states *what\n");
    out.push_str("// each part requires*. A region below became a residency constraint and is\n");
    out.push_str("// marked TODO: confirm it is a requirement, not just where it happened to run.\n\n");
    out.push_str("workload terraform-import {\n");
    out.push_str("  profile = balanced\n");

    for r in resources.iter().filter(|r| r.data_ident.is_some()) {
        let id = r.data_ident.as_ref().unwrap();
        out.push_str(&format!("\n  data {} {{   // {}.{}\n", id, r.ty, r.name));
        if let Some(m) = &r.multiplicity {
            push_multiplicity(&mut out, m);
        }
        out.push_str("    sensitivity = S2            // TODO: confirm (S0 public .. S3 regulated)\n");
        if let Some((res, raw)) = &r.residency {
            out.push_str(&format!(
                "    residency   = {}            // TODO: confirm; from {}\n",
                res, raw
            ));
        }
        out.push_str("  }\n");
    }

    for r in resources.iter().filter(|r| matches!(r.kind, Kind::Compute | Kind::Store | Kind::Static)) {
        let mut header = format!("{}.{}", r.ty, r.name);
        if let Some(s) = &r.size {
            header.push_str(&format!(" ({})", s));
        }
        out.push_str(&format!("\n  service {} : {} {{   // {}\n", r.ident, r.class, header));
        if let Some(m) = &r.multiplicity {
            push_multiplicity(&mut out, m);
        }
        if let Some((res, raw)) = &r.residency {
            out.push_str(&format!(
                "    residency  = {}             // TODO: confirm; from {}\n",
                res, raw
            ));
        }
        if r.class == "backend-api" {
            out.push_str("    // slo.p95 <= 200 ms          // TODO: set a latency target\n");
        }
        let mut deps = deps_of(r);
        if let Some(own) = &r.data_ident {
            if !deps.contains(own) {
                deps.insert(0, own.clone());
            }
        }
        for d in deps {
            out.push_str(&format!("    depends_on = {}\n", d));
        }
        out.push_str("  }\n");
    }

    out.push_str("}\n");
    Ok(Imported { bwk: out, notes })
}
