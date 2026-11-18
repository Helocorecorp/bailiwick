//! `docker-compose.yml` → BWK. Compose describes *how* containers are
//! wired together on one host; BWK describes *what each part requires*.
//! The importer recovers the service graph, which services are stateful
//! stores (those become `data` blocks that dependents point at), images,
//! and `deploy.resources` sizing — and leaves residency, sensitivity and
//! SLOs as explicit TODOs.

use crate::ast::Resources;

use super::yaml_min::{self, Node};
use super::{classify_image, parse_cpu, parse_mem_gb, resources_line, sanitize_ident, Imported};

struct Svc {
    ident: String,
    image: Option<String>,
    class: &'static str,
    stateful: bool,
    deps: Vec<String>,
    named_volumes: Vec<String>,
    resources: Resources,
}

/// `deploy.resources.reservations` (preferred) or `.limits`.
fn deploy_resources(def: &Node) -> Resources {
    let mut r = Resources::default();
    let res = match def.get("deploy").and_then(|d| d.get("resources")) {
        Some(x) => x,
        None => return r,
    };
    let reservations = res.get("reservations");
    let limits = res.get("limits");
    let pick = |key: &str| -> Option<&str> {
        reservations
            .and_then(|q| q.get(key))
            .or_else(|| limits.and_then(|l| l.get(key)))
            .and_then(|v| v.as_str())
    };
    r.vcpu = pick("cpus").and_then(parse_cpu);
    r.mem_gb = pick("memory").and_then(|m| parse_mem_gb(m, false));
    r
}

pub fn import(src: &str) -> Result<Imported, String> {
    let root = yaml_min::parse(src)?;
    let mut notes = Vec::new();

    let services = root
        .get("services")
        .and_then(|n| n.as_map())
        .ok_or_else(|| "compose file has no `services:` mapping".to_string())?;
    if services.is_empty() {
        return Err("compose file has an empty `services:` mapping".to_string());
    }

    let name_raw = root.get("name").and_then(|n| n.as_str()).unwrap_or("app");
    let name = sanitize_ident(name_raw);

    // First pass: classify.
    let mut svcs: Vec<Svc> = Vec::new();
    for (sname, def) in services {
        let image = def.get("image").and_then(|n| n.as_str()).map(|s| s.to_string());
        let (class, stateful) = classify_image(sname, image.as_deref());
        let deps: Vec<String> = match def.get("depends_on") {
            Some(Node::Seq(items)) => items
                .iter()
                .filter_map(|n| n.as_str().map(|s| s.to_string()))
                .collect(),
            Some(Node::Map(m)) => m.iter().map(|(k, _)| k.clone()).collect(),
            _ => Vec::new(),
        };
        let named_volumes: Vec<String> = def
            .get("volumes")
            .and_then(|n| n.as_seq())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|n| n.as_str())
                    .filter_map(|v| {
                        let src = v.split(':').next().unwrap_or("");
                        // Named volumes, not bind mounts (./path or /path).
                        if src.is_empty() || src.starts_with('.') || src.starts_with('/') {
                            None
                        } else {
                            Some(sanitize_ident(src))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ident = sanitize_ident(sname);
        if &ident != sname {
            notes.push(format!("service `{}` renamed `{}` to be a valid identifier", sname, ident));
        }
        let resources = deploy_resources(def);
        svcs.push(Svc { ident, image, class, stateful, deps, named_volumes, resources });
    }

    // Data blocks: one per stateful store, plus one per named volume on a
    // non-store service.
    let store_ident = |s: &Svc| format!("{}-store", s.ident);
    let mut data_blocks: Vec<(String, String)> = Vec::new(); // (ident, comment)
    for s in &svcs {
        if s.stateful {
            let comment = match &s.image {
                Some(i) => format!("state held by service `{}` (image {})", s.ident, i),
                None => format!("state held by service `{}`", s.ident),
            };
            data_blocks.push((store_ident(s), comment));
        } else {
            for v in &s.named_volumes {
                let id = format!("{}-{}", s.ident, v);
                if !data_blocks.iter().any(|(d, _)| d == &id) {
                    data_blocks.push((id, format!("named volume `{}` on service `{}`", v, s.ident)));
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str("// Imported from docker-compose.yml by `bailiwick import compose`.\n");
    out.push_str("// Compose says *how* containers are wired; BWK states *what each part\n");
    out.push_str("// requires*. Residency, sensitivity and SLOs cannot be inferred from\n");
    out.push_str("// compose — every TODO below needs a human decision.\n\n");
    out.push_str(&format!("workload {} {{\n", name));
    out.push_str("  profile = balanced\n");
    out.push_str("  // residency: add `residency = eu|us|ca|uk|apac` on data and services as required\n");

    for (id, comment) in &data_blocks {
        out.push_str(&format!("\n  data {} {{   // {}\n", id, comment));
        out.push_str("    sensitivity = S2            // TODO: confirm (S0 public .. S3 regulated)\n");
        out.push_str("  }\n");
    }

    for s in &svcs {
        let header = match &s.image {
            Some(_) => String::new(),
            None => "   // built from source".to_string(),
        };
        out.push_str(&format!("\n  service {} : {} {{{}\n", s.ident, s.class, header));
        if let Some(i) = &s.image {
            out.push_str(&format!("    image = \"{}\"\n", i.replace('"', "\\\"")));
        }
        if s.class == "backend-api" {
            out.push_str("    // slo.p95 <= 200 ms          // TODO: set a latency target\n");
        }
        if let Some(line) = resources_line(&s.resources) {
            out.push_str(&format!("    {}   // from deploy.resources\n", line));
        }
        if s.stateful {
            out.push_str(&format!("    depends_on = {}\n", store_ident(s)));
        } else {
            for v in &s.named_volumes {
                out.push_str(&format!("    depends_on = {}-{}\n", s.ident, v));
            }
        }
        for d in &s.deps {
            let dep_ident = sanitize_ident(d);
            match svcs.iter().find(|x| x.ident == dep_ident) {
                Some(target) if target.stateful => {
                    out.push_str(&format!(
                        "    depends_on = {}   // compose depends_on: {}\n",
                        store_ident(target),
                        d
                    ));
                }
                Some(_) => {
                    out.push_str(&format!(
                        "    // compose depends_on: {} (service-to-service; BWK v0 tracks data dependencies only)\n",
                        d
                    ));
                }
                None => {
                    notes.push(format!(
                        "service `{}` depends_on `{}`, which is not defined in this compose file",
                        s.ident, d
                    ));
                }
            }
        }
        out.push_str("  }\n");
    }

    out.push_str("}\n");
    Ok(Imported { bwk: out, notes })
}
