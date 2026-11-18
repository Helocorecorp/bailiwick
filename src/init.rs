//! `bailiwick init`: give a project its `bailiwick.bwk` in one command.
//!
//! It looks through the project for the deployment files it already has —
//! `fly.toml`, Docker Compose, Kubernetes manifests, Terraform — runs the
//! matching importers, and writes one file with every workload they found,
//! each inferred requirement marked `TODO: confirm`. A project with none of
//! those gets a commented starting point instead. Either way the result
//! checks, and lists the rule packs to opt into.

use std::path::{Path, PathBuf};

use crate::import;

/// What `init` produced.
pub struct Init {
    pub bwk: String,
    /// The files each importer read, relative to the project.
    pub sources: Vec<String>,
    /// Things the importers could not infer, to confirm by hand.
    pub notes: Vec<String>,
}

const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "dist", "build", ".terraform", "__pycache__"];
const MAX_DEPTH: usize = 4;

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if depth < MAX_DEPTH && !name.starts_with('.') && !SKIP_DIRS.contains(&name) {
                walk(&p, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn is_compose(name: &str) -> bool {
    matches!(name, "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml")
}

/// A YAML file that holds Kubernetes objects.
fn is_k8s(src: &str) -> bool {
    let has = |k: &str| src.lines().any(|l| l.starts_with(k));
    has("apiVersion:") && has("kind:")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// Give every workload a distinct name: two Terraform directories both
/// import as `terraform-import`.
fn dedupe_workloads(bwk: &mut String, taken: &mut Vec<String>) {
    let mut out = String::new();
    for line in bwk.lines() {
        if let Some(rest) = line.strip_prefix("workload ") {
            if let Some(name) = rest.strip_suffix(" {") {
                let mut n = name.to_string();
                let mut i = 2;
                while taken.contains(&n) {
                    n = format!("{}-{}", name, i);
                    i += 1;
                }
                taken.push(n.clone());
                out.push_str(&format!("workload {} {{\n", n));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    *bwk = out;
}

const PACKS_HINT: &str = "\
// Rule packs: uncomment the laws that apply to you. Each pack is a policy;
// policies only narrow, so importing more can never loosen anything.
// import \"gdpr\"               // EU: EEA or adequate countries
// import \"eu-sovereign\"       // EU, beyond the reach of US/CN/RU law
// import \"uk-gdpr\"            // UK: UK or UK-adequate countries
// import \"pipeda\"             // Canada (federal)
// import \"canada-residency\"   // Canada only
// import \"quebec-law25\"       // Québec only
// import \"hipaa\"              // US health data
// import \"ccpa\"               // California
// import \"lgpd\"               // Brazil
// import \"pipl\"               // China
// import \"rbi-payments\"       // India, payment data
// import \"my-health-records\"  // Australia, My Health Record
";

const STARTER: &str = "\
// What this project needs. Describe requirements, never servers: where its
// data may live, how sensitive it is, how fast it must answer, how many
// providers it must survive losing. Then run `bailiwick explain bailiwick.bwk`
// to see where each part may run, and why everywhere else is ruled out.

workload app {
  data users {
    sensitivity = S2            // S0 public .. S3 regulated
    residency   = eu            // global, eu, eea, us, ca, uk, ...
  }

  service api : backend-api {
    // slo.p95 <= 200 ms from eu  // a latency rule: only offers with a measured p95
    //                            // from the EU within 200 ms qualify; unmeasured fail closed
    redundancy { vendors >= 2 }
    resources  { vcpu >= 1  mem >= 1 GB }
    depends_on  = users         // inherits the data's residency and sensitivity
  }
}
";

/// Build a `bailiwick.bwk` for the project at `root`.
pub fn init(root: &Path) -> Result<Init, String> {
    if !root.is_dir() {
        return Err(format!("`{}` is not a directory", root.display()));
    }
    let mut files = Vec::new();
    walk(root, 0, &mut files);

    let mut parts: Vec<String> = Vec::new();
    let mut sources = Vec::new();
    let mut notes = Vec::new();
    let mut taken = Vec::new();
    let mut add = |what: &str, src_files: Vec<String>, result: Result<import::Imported, String>, notes: &mut Vec<String>| {
        match result {
            Ok(mut imp) => {
                dedupe_workloads(&mut imp.bwk, &mut taken);
                let shown = src_files.join(", ");
                notes.extend(imp.notes.into_iter().map(|n| format!("{}: {}", shown, n)));
                parts.push(format!("// From {} ({}).\n{}", shown, what, imp.bwk));
                sources.extend(src_files);
            }
            Err(e) => notes.push(format!("{} ({}): not imported: {}", src_files.join(", "), what, e)),
        }
    };

    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();

    // fly.toml and Compose files: one import each.
    for p in &files {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "fly.toml" {
            add("Fly.io", vec![rel(root, p)], import::fly::import(&read(p)), &mut notes);
        } else if is_compose(name) {
            add("Docker Compose", vec![rel(root, p)], import::compose::import(&read(p)), &mut notes);
        }
    }

    // Terraform: one import per directory, since a module spans its files.
    let mut tf_dirs: Vec<PathBuf> = Vec::new();
    for p in &files {
        if p.extension().and_then(|e| e.to_str()) == Some("tf") {
            let d = p.parent().unwrap_or(root).to_path_buf();
            if !tf_dirs.contains(&d) {
                tf_dirs.push(d);
            }
        }
    }
    for d in &tf_dirs {
        let mine: Vec<&PathBuf> = files
            .iter()
            .filter(|p| p.parent() == Some(d.as_path()) && p.extension().and_then(|e| e.to_str()) == Some("tf"))
            .collect();
        let src: String = mine.iter().map(|p| read(p) + "\n").collect();
        add("Terraform", mine.iter().map(|p| rel(root, p)).collect(), import::terraform::import(&src), &mut notes);
    }

    // Kubernetes: every manifest together, as one cluster's worth.
    let k8s: Vec<&PathBuf> = files
        .iter()
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            (ext == "yaml" || ext == "yml") && !is_compose(name) && is_k8s(&read(p))
        })
        .collect();
    if !k8s.is_empty() {
        let src: String = k8s.iter().map(|p| format!("---\n{}\n", read(p))).collect();
        add("Kubernetes", k8s.iter().map(|p| rel(root, p)).collect(), import::k8s::import(&src), &mut notes);
    }

    let mut bwk = String::from(PACKS_HINT);
    bwk.push('\n');
    if parts.is_empty() {
        bwk.push_str(STARTER);
    } else {
        bwk.push_str(&parts.join("\n"));
    }
    Ok(Init { bwk, sources, notes })
}
