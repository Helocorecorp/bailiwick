//! Kubernetes manifests → BWK. Kubernetes describes *how* pods run on one
//! cluster; BWK describes *what each part requires*. The importer recovers
//! workloads (Deployments, StatefulSets, Jobs, CronJobs), their images and
//! resource requests, the volumes they depend on, and — only where a
//! nodeSelector names a cloud region — a residency to confirm.

use crate::ast::Resources;

use super::yaml_min::{self, Node};
use super::{
    any_cloud_region_to_residency, classify_image, duration_literal_from_seconds, parse_cpu,
    parse_mem_gb, resources_line, sanitize_ident, Imported,
};

struct Unit {
    kind: &'static str, // "service" | "job"
    ident: String,
    class: &'static str,
    image: Option<String>,
    resources: Resources,
    deps: Vec<String>,
    residency: Option<(&'static str, String)>, // (region, source label value)
    comments: Vec<String>,
    deadline_s: Option<f64>,
}

/// Image and resource requests of the first container in a pod spec.
fn pod_main_container(pod: Option<&Node>) -> (Option<String>, Resources) {
    let c = pod
        .and_then(|p| p.get("containers"))
        .and_then(|c| c.as_seq())
        .and_then(|s| s.first());
    let image = c
        .and_then(|c| c.get("image"))
        .and_then(|i| i.as_str())
        .map(|s| s.to_string());
    let mut r = Resources::default();
    if let Some(res) = c.and_then(|c| c.get("resources")) {
        let req = res.get("requests");
        let lim = res.get("limits");
        let pick = |key: &str| -> Option<&str> {
            req.and_then(|q| q.get(key))
                .or_else(|| lim.and_then(|l| l.get(key)))
                .and_then(|v| v.as_str())
        };
        r.vcpu = pick("cpu").and_then(parse_cpu);
        r.mem_gb = pick("memory").and_then(|m| parse_mem_gb(m, true));
        r.gpu = lim
            .and_then(|l| l.get("nvidia.com/gpu"))
            .or_else(|| req.and_then(|q| q.get("nvidia.com/gpu")))
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite());
    }
    (image, r)
}

fn pod_region(pod: Option<&Node>) -> Option<String> {
    let ns = pod?.get("nodeSelector")?;
    ns.get("topology.kubernetes.io/region")
        .or_else(|| ns.get("failure-domain.beta.kubernetes.io/region"))
        .and_then(|r| r.as_str())
        .map(|s| s.to_string())
}

/// Record a `data` block, keeping the names unique. Two Kubernetes objects
/// can reduce to one BWK name — a PersistentVolumeClaim called `db-store`
/// beside a StatefulSet called `db` — and a workload cannot hold two data
/// blocks under one name, so the output would not check. The first name
/// wins, and the clash is reported so that the second store is not lost in
/// silence.
fn add_data_block(
    data_blocks: &mut Vec<(String, String)>,
    notes: &mut Vec<String>,
    ident: String,
    comment: String,
) {
    if let Some((_, kept)) = data_blocks.iter().find(|(id, _)| *id == ident) {
        notes.push(format!(
            "two objects both become the data block `{}` ({}, and {}); only the first is \
             written, so rename one of them if they are separate stores",
            ident, kept, comment
        ));
        return;
    }
    data_blocks.push((ident, comment));
}

pub fn import(src: &str) -> Result<Imported, String> {
    let docs = yaml_min::parse_all(src)?;
    if docs.is_empty() {
        return Err("no YAML documents found".to_string());
    }
    let mut notes = Vec::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut data_blocks: Vec<(String, String)> = Vec::new();
    let mut namespace: Option<String> = None;
    let mut ignored: Vec<String> = Vec::new();

    for doc in &docs {
        let kind = doc.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let meta = doc.get("metadata");
        let name = meta
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unnamed");
        if namespace.is_none() {
            if let Some(ns) = meta.and_then(|m| m.get("namespace")).and_then(|n| n.as_str()) {
                namespace = Some(ns.to_string());
            }
        }
        let ident = sanitize_ident(name);

        match kind {
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" => {
                let pod = doc
                    .get("spec")
                    .and_then(|s| s.get("template"))
                    .and_then(|t| t.get("spec"));
                let (image, resources) = pod_main_container(pod);
                let stateful = kind == "StatefulSet";
                let (class, store) = classify_image(name, image.as_deref());
                let class = if stateful { "database" } else { class };
                let mut comments = vec![format!("{} {}", kind, name)];
                if let Some(r) = doc
                    .get("spec")
                    .and_then(|s| s.get("replicas"))
                    .and_then(|r| r.as_str())
                {
                    comments.push(format!(
                        "replicas: {} — replicas are not vendors; add `redundancy {{ vendors >= n }}` if needed",
                        r
                    ));
                }
                let mut deps = Vec::new();
                if stateful || store {
                    let d = format!("{}-store", ident);
                    let what = format!("state of {} `{}`", kind, name);
                    add_data_block(&mut data_blocks, &mut notes, d.clone(), what);
                    deps.push(d);
                }
                if let Some(vols) = pod.and_then(|p| p.get("volumes")).and_then(|v| v.as_seq()) {
                    for v in vols {
                        if let Some(claim) = v
                            .get("persistentVolumeClaim")
                            .and_then(|p| p.get("claimName"))
                            .and_then(|c| c.as_str())
                        {
                            let d = sanitize_ident(claim);
                            if !deps.contains(&d) {
                                deps.push(d);
                            }
                        }
                    }
                }
                let residency = pod_region(pod).and_then(|label| {
                    match any_cloud_region_to_residency(&label) {
                        Some(r) => Some((r, label)),
                        None => {
                            notes.push(format!(
                                "nodeSelector region `{}` has no BWK residency region",
                                label
                            ));
                            None
                        }
                    }
                });
                units.push(Unit {
                    kind: "service",
                    ident,
                    class,
                    image,
                    resources,
                    deps,
                    residency,
                    comments,
                    deadline_s: None,
                });
            }
            "PersistentVolumeClaim" => {
                let storage = doc
                    .get("spec")
                    .and_then(|s| s.get("resources"))
                    .and_then(|r| r.get("requests"))
                    .and_then(|r| r.get("storage"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("?");
                let what = format!("PersistentVolumeClaim `{}` ({})", name, storage);
                add_data_block(&mut data_blocks, &mut notes, ident, what);
            }
            "Job" | "CronJob" => {
                let (pod, deadline, schedule) = if kind == "CronJob" {
                    let js = doc
                        .get("spec")
                        .and_then(|s| s.get("jobTemplate"))
                        .and_then(|j| j.get("spec"));
                    (
                        js.and_then(|s| s.get("template")).and_then(|t| t.get("spec")),
                        js.and_then(|s| s.get("activeDeadlineSeconds")),
                        doc.get("spec").and_then(|s| s.get("schedule")).and_then(|s| s.as_str()),
                    )
                } else {
                    (
                        doc.get("spec").and_then(|s| s.get("template")).and_then(|t| t.get("spec")),
                        doc.get("spec").and_then(|s| s.get("activeDeadlineSeconds")),
                        None,
                    )
                };
                let (image, resources) = pod_main_container(pod);
                let mut comments = vec![format!("{} {}", kind, name)];
                if let Some(s) = schedule {
                    comments.push(format!("schedule: {}", s));
                }
                let deadline_s = deadline
                    .and_then(|d| d.as_str())
                    .and_then(|d| d.parse::<f64>().ok())
                    .filter(|d| d.is_finite());
                let residency = pod_region(pod).and_then(|label| {
                    any_cloud_region_to_residency(&label).map(|r| (r, label))
                });
                units.push(Unit {
                    kind: "job",
                    ident,
                    class: "background-jobs",
                    image,
                    resources,
                    deps: Vec::new(),
                    residency,
                    comments,
                    deadline_s,
                });
            }
            "" => notes.push("a document without `kind` was skipped".to_string()),
            other => ignored.push(format!("{} {}", other, name)),
        }
    }

    if units.is_empty() && data_blocks.is_empty() {
        return Err(
            "no Deployments, StatefulSets, Jobs, CronJobs or PersistentVolumeClaims found"
                .to_string(),
        );
    }
    // A volume referenced but not defined in this manifest still exists.
    for u in &units {
        for d in &u.deps {
            if !data_blocks.iter().any(|(id, _)| id == d) {
                data_blocks.push((
                    d.clone(),
                    "referenced PersistentVolumeClaim (not defined in this manifest)".to_string(),
                ));
            }
        }
    }
    if !ignored.is_empty() {
        notes.push(format!(
            "ignored {} object(s) with no BWK equivalent: {}",
            ignored.len(),
            ignored.join(", ")
        ));
    }

    let wl = namespace
        .map(|n| sanitize_ident(&n))
        .unwrap_or_else(|| "k8s-app".to_string());

    let mut out = String::new();
    out.push_str("// Imported from Kubernetes manifests by `bailiwick import k8s`.\n");
    out.push_str("// Kubernetes says *how* pods run on one cluster; BWK states *what each\n");
    out.push_str("// part requires*. Residency comes only from nodeSelector region labels;\n");
    out.push_str("// everything marked TODO needs a human decision.\n\n");
    out.push_str(&format!("workload {} {{\n", wl));
    out.push_str("  profile = balanced\n");

    for (id, comment) in &data_blocks {
        out.push_str(&format!("\n  data {} {{   // {}\n", id, comment));
        out.push_str("    sensitivity = S2            // TODO: confirm (S0 public .. S3 regulated)\n");
        out.push_str("  }\n");
    }

    for u in &units {
        out.push_str(&format!(
            "\n  {} {} : {} {{   // {}\n",
            u.kind,
            u.ident,
            u.class,
            u.comments.join("; ")
        ));
        if let Some(i) = &u.image {
            out.push_str(&format!("    image = \"{}\"\n", i.replace('"', "\\\"")));
        }
        if let Some((res, label)) = &u.residency {
            out.push_str(&format!(
                "    residency = {}             // TODO: confirm; from nodeSelector region = {}\n",
                res, label
            ));
        }
        if u.kind == "job" {
            match u.deadline_s {
                Some(s) => out.push_str(&format!(
                    "    deadline = {}             // from activeDeadlineSeconds\n",
                    duration_literal_from_seconds(s)
                )),
                None => out.push_str("    // deadline = 6 h             // TODO: set a deadline\n"),
            }
            out.push_str("    checkpointable = false      // TODO: confirm\n");
        } else if u.class == "backend-api" {
            out.push_str("    // slo.p95 <= 200 ms          // TODO: set a latency target\n");
        }
        if let Some(line) = resources_line(&u.resources) {
            out.push_str(&format!("    {}\n", line));
        }
        for d in &u.deps {
            out.push_str(&format!("    depends_on = {}\n", d));
        }
        out.push_str("  }\n");
    }

    out.push_str("}\n");
    Ok(Imported { bwk: out, notes })
}
