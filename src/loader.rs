//! File loading with `import` resolution.
//!
//! An imported file contributes its policies and offers (and, transitively,
//! its own imports). Imported workloads are ignored with a warning — a
//! workload is deployed from the file that declares it, never by being
//! imported. Imports resolve relative to the importing file; cycles are
//! detected and reported rather than looping.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ast::{File, Policy};
use crate::check;
use crate::diag::Diag;
use crate::parser;

pub struct Loaded {
    pub file: File,
    pub diags: Vec<Diag>,
}

fn prefix_diags(diags: Vec<Diag>, label: &str) -> Vec<Diag> {
    diags
        .into_iter()
        .map(|mut d| {
            d.msg = format!("{}: {}", label, d.msg);
            d
        })
        .collect()
}

/// How the loader reads an imported file: from disk, or (in the
/// WebAssembly build) from files the host supplied.
pub type Reader<'a> = &'a dyn Fn(&Path) -> std::io::Result<String>;

fn disk(p: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(p)
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Parse `src` (the contents of `path`) and recursively merge its imports.
/// Does *not* run checks; callers check the merged result once.
fn load_rec_src(
    path: &Path,
    src: &str,
    visited: &mut HashSet<PathBuf>,
    is_root: bool,
    read: Reader,
) -> (File, Vec<Diag>) {
    let mut diags = Vec::new();
    visited.insert(canon(path));

    let parsed = parser::parse(src);
    let mut file = parsed.file;
    if is_root {
        diags.extend(parsed.diags);
    } else {
        diags.extend(prefix_diags(parsed.diags, &path.display().to_string()));
        if !file.workloads.is_empty() {
            diags.push(Diag::warning(
                0,
                0,
                format!(
                    "`{}` declares {} workload(s); imported workloads are ignored (only policies and offers are imported)",
                    path.display(),
                    file.workloads.len()
                ),
            ));
            file.workloads.clear();
        }
    }

    let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let imports = file.imports.clone();
    for imp in &imports {
        if crate::builtin::is_pack_name(&imp.path) {
            let pseudo = PathBuf::from(format!("<pack {}>", imp.path));
            if visited.contains(&pseudo) {
                continue;
            }
            match crate::builtin::pack(&imp.path) {
                Some(src) => {
                    let (sub, sub_diags) = load_rec_src(&pseudo, src, visited, false, read);
                    diags.extend(sub_diags);
                    file.policies.extend(sub.policies);
                }
                None => diags.push(Diag::error(
                    imp.line,
                    imp.col,
                    format!(
                        "no rule pack named `{}` (available: {}); to import a file, give its path, e.g. `{}.bwk`",
                        imp.path,
                        crate::builtin::pack_names(),
                        imp.path
                    ),
                )),
            }
            continue;
        }
        let target = base.join(&imp.path);
        if visited.contains(&canon(&target)) {
            diags.push(Diag::warning(
                imp.line,
                imp.col,
                format!("import `{}` skipped: already loaded (cycle or duplicate)", imp.path),
            ));
            continue;
        }
        let (sub, sub_diags) = load_rec(&target, visited, false, read);
        diags.extend(sub_diags);
        file.policies.extend(sub.policies);
        file.offers.extend(sub.offers);
    }

    (file, diags)
}

/// Read `path` and load it with `load_rec_src`.
fn load_rec(path: &Path, visited: &mut HashSet<PathBuf>, is_root: bool, read: Reader) -> (File, Vec<Diag>) {
    let src = match read(path) {
        Ok(s) => s,
        Err(e) => {
            return (
                File::default(),
                vec![Diag::error(0, 0, format!("cannot read `{}`: {}", path.display(), e))],
            );
        }
    };
    load_rec_src(path, &src, visited, is_root, read)
}

fn finish(mut file: File, mut diags: Vec<Diag>, extra_policies: Vec<Policy>) -> Loaded {
    file.policies.extend(extra_policies);
    diags.extend(check::check(&file));
    Loaded { file, diags }
}

/// Load a file, resolve its imports, and run semantic checks on the merged
/// result. Extra policies (e.g. from `--policy` flags) are merged in
/// before checking, so an externally imposed policy is enforced exactly
/// like an imported one.
pub fn load(path: &Path, extra_policies: Vec<Policy>) -> Loaded {
    let mut visited = HashSet::new();
    let (file, diags) = load_rec(path, &mut visited, true, &disk);
    finish(file, diags, extra_policies)
}

/// Like `load`, but with the root file's contents supplied by the caller
/// (an editor buffer, say) rather than read from disk. Imports are still
/// resolved relative to `path` and read from disk.
pub fn load_source(path: &Path, src: &str, extra_policies: Vec<Policy>) -> Loaded {
    load_source_with(path, src, extra_policies, &disk)
}

/// Like `load_source`, with imports read by `read` instead of from disk.
pub fn load_source_with(path: &Path, src: &str, extra_policies: Vec<Policy>, read: Reader) -> Loaded {
    let mut visited = HashSet::new();
    let (file, diags) = load_rec_src(path, src, &mut visited, true, read);
    finish(file, diags, extra_policies)
}

/// Load policy files (for `--policy`). Each is loaded with its own imports
/// resolved; only the policies are kept.
pub fn load_policies(paths: &[String]) -> (Vec<Policy>, Vec<Diag>) {
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for p in paths {
        let mut visited = HashSet::new();
        let (file, d) = match crate::builtin::pack(p) {
            Some(src) if crate::builtin::is_pack_name(p) && !Path::new(p).exists() => {
                load_rec_src(Path::new(&format!("<pack {}>", p)), src, &mut visited, false, &disk)
            }
            _ => load_rec(Path::new(p), &mut visited, false, &disk),
        };
        diags.extend(d);
        if file.policies.is_empty() {
            diags.push(Diag::warning(0, 0, format!("policy file `{}` defines no policies", p)));
        }
        out.extend(file.policies);
    }
    (out, diags)
}
