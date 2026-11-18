//! `fly.toml` → BWK. Fly describes *where* an app runs (a primary region,
//! a VM size); BWK describes *what it requires*. The importer turns the
//! region into a residency constraint and flags it for confirmation,
//! because "it ran in fra" is not the same statement as "it must stay in
//! the EU". VM sizing becomes `resources` minimums.

use crate::ast::Resources;

use super::{
    classify_image, fly_region_to_residency, fmt_num, parse_mem_gb, resources_line,
    sanitize_ident, toml_min, Imported,
};

/// Human-readable rendering of a scalar TOML value for notes.
fn toml_display(v: &toml_min::Value) -> String {
    match v {
        toml_min::Value::Str(s) => s.clone(),
        toml_min::Value::Int(i) => i.to_string(),
        toml_min::Value::Float(f) => fmt_num(*f),
        toml_min::Value::Bool(b) => b.to_string(),
        toml_min::Value::Array(_) => "[...]".to_string(),
        toml_min::Value::Table(_) => "{...}".to_string(),
    }
}

/// `[[vm]]` → resources. `size = "shared-cpu-2x"` implies 2 vCPUs when
/// `cpus` is absent; `memory` may be `"1gb"`, `"512mb"` or an integer in MB.
fn vm_resources(vm: &toml_min::Table) -> Resources {
    let get = |k: &str| vm.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
    let mut r = Resources { vcpu: get("cpus").and_then(|v| v.as_i64()).map(|n| n as f64), ..Default::default() };
    if r.vcpu.is_none() {
        if let Some(size) = get("size").and_then(|v| v.as_str()) {
            // "...-Nx"
            if let Some(stripped) = size.strip_suffix('x') {
                if let Some(idx) = stripped.rfind('-') {
                    let n = stripped[idx + 1..].parse::<f64>().ok();
                    r.vcpu = n.filter(|v| v.is_finite());
                }
            }
        }
    }
    r.mem_gb = match get("memory") {
        Some(toml_min::Value::Str(s)) => parse_mem_gb(s, false),
        Some(toml_min::Value::Int(mb)) => Some((*mb as f64) / 1024.0),
        _ => None,
    };
    r
}

pub fn import(src: &str) -> Result<Imported, String> {
    let root = toml_min::parse(src)?;
    let mut notes = Vec::new();

    let app_raw = root.get("app").and_then(|v| v.as_str()).unwrap_or("app");
    let app = sanitize_ident(app_raw);
    if app != app_raw {
        notes.push(format!("app name `{}` was renamed `{}` to be a valid identifier", app_raw, app));
    }

    let region = root.get("primary_region").and_then(|v| v.as_str());
    let residency = match region {
        Some(r) => match fly_region_to_residency(r) {
            Some(res) => Some((res, r.to_string())),
            None => {
                notes.push(format!(
                    "fly region `{}` has no BWK residency region; residency left unconstrained",
                    r
                ));
                None
            }
        },
        None => {
            notes.push("no primary_region in fly.toml; residency left unconstrained".to_string());
            None
        }
    };

    // Mounts may be `[mounts]` (one) or `[[mounts]]` (many).
    let mut mounts: Vec<(String, String)> = Vec::new();
    if let Some(m) = root.get("mounts") {
        let tables: Vec<&toml_min::Table> = match m {
            toml_min::Value::Array(items) => items.iter().filter_map(|v| v.as_table()).collect(),
            toml_min::Value::Table(t) => vec![t],
            _ => Vec::new(),
        };
        for t in tables {
            let get = |k: &str| t.iter().find(|(kk, _)| kk == k).and_then(|(_, v)| v.as_str());
            if let Some(source) = get("source") {
                mounts.push((
                    sanitize_ident(source),
                    get("destination").unwrap_or("?").to_string(),
                ));
            }
        }
    }

    let image = root
        .get("build")
        .and_then(|b| b.get("image"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let (class, _) = classify_image(&app, image.as_deref());
    let has_http = root.get("http_service").is_some() || root.get("services").is_some();
    let internal_port = root
        .get("http_service")
        .and_then(|h| h.get("internal_port"))
        .and_then(|v| v.as_i64());
    let statics: Vec<(String, String)> = root
        .get("statics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_table())
                .map(|t| {
                    let get = |k: &str| {
                        t.iter().find(|(kk, _)| kk == k).and_then(|(_, v)| v.as_str()).unwrap_or("?")
                    };
                    (get("guest_path").to_string(), get("url_prefix").to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // Sizing: first [[vm]] entry (or a [vm] table).
    let vm_table: Option<&toml_min::Table> = match root.get("vm") {
        Some(toml_min::Value::Array(items)) => items.first().and_then(|t| t.as_table()),
        Some(toml_min::Value::Table(t)) => Some(t),
        _ => None,
    };
    let resources = vm_table.map(vm_resources).unwrap_or_default();
    if let Some(t) = vm_table {
        let get = |k: &str| t.iter().find(|(kk, _)| kk == k).map(|(_, v)| toml_display(v));
        notes.push(format!(
            "resources inferred from [[vm]]: size={} memory={} cpus={}",
            get("size").unwrap_or_else(|| "-".into()),
            get("memory").unwrap_or_else(|| "-".into()),
            get("cpus").unwrap_or_else(|| "-".into())
        ));
    }

    let mut out = String::new();
    out.push_str("// Imported from fly.toml by `bailiwick import fly`.\n");
    out.push_str("// Fly says *where* this ran; BWK states *what is required*. Every\n");
    out.push_str("// constraint below marked TODO was inferred and must be confirmed.\n\n");
    out.push_str(&format!("workload {} {{\n", app));
    out.push_str("  profile = balanced\n");

    for (src, dest) in &mounts {
        out.push_str(&format!("\n  data {} {{\n", src));
        out.push_str(&format!(
            "    sensitivity = S2            // TODO: confirm (fly volume mounted at {})\n",
            dest
        ));
        if let Some((res, fly)) = &residency {
            out.push_str(&format!(
                "    residency   = {}            // TODO: confirm; from primary_region = \"{}\"\n",
                res, fly
            ));
        }
        out.push_str("  }\n");
    }

    // Emit the main service when fly serves HTTP, or when there is nothing
    // else to emit (an app with no mounts and no statics is still an app).
    if has_http || (mounts.is_empty() && statics.is_empty()) {
        let port_note = internal_port
            .map(|p| format!(" (internal_port {})", p))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n  service {} : {} {{   // from [http_service]{}\n",
            app, class, port_note
        ));
        if let Some(i) = &image {
            out.push_str(&format!("    image      = \"{}\"\n", i.replace('"', "\\\"")));
        }
        if let Some((res, fly)) = &residency {
            out.push_str(&format!(
                "    residency  = {}             // TODO: confirm; from primary_region = \"{}\"\n",
                res, fly
            ));
        }
        if class == "backend-api" {
            out.push_str("    // slo.p95 <= 200 ms          // TODO: set a latency target\n");
        }
        if let Some(line) = resources_line(&resources) {
            out.push_str(&format!("    {}   // from [[vm]]\n", line));
        }
        for (src, _) in &mounts {
            out.push_str(&format!("    depends_on = {}\n", src));
        }
        out.push_str("  }\n");
    }

    if !statics.is_empty() {
        out.push_str(&format!("\n  service {}-static : frontend-static {{   // from [[statics]]\n", app));
        for (guest, prefix) in &statics {
            out.push_str(&format!("    // {} served at {}\n", guest, prefix));
        }
        if let Some((res, _)) = &residency {
            out.push_str(&format!("    residency = {}\n", res));
        }
        out.push_str("  }\n");
    }

    out.push_str("}\n");
    Ok(Imported { bwk: out, notes })
}
