//! The real Bailiwick toolchain, compiled to WebAssembly: the same parser,
//! checker, evaluator and rule packs as the command line, nothing
//! reimplemented in JavaScript or Python.
//!
//! Two entry points, both taking UTF-8 written into memory from
//! `bwk_alloc` and returning a pointer to a JSON result whose length is
//! `bwk_result_len()`:
//!
//! - `bwk_run(src)`: the browser playground. One program; its own offers
//!   if it declares any, else the built-in catalog.
//!   `{ "diagnostics": [...], "formatted": "...", "report": "...",
//!      "plan": {...}, "certificate": {...}, "catalog": "own" | "example" }`
//!   With no offers of its own, a program runs against the fictional
//!   example offers, and the page says so.
//! - `bwk_call(request)`: a command, for the npm and Python packages. The
//!   host reads the files (the main file and everything it imports) and
//!   passes them in; the result is what the native CLI would print.
//!   Request: `{ "command": "check" | "explain" | "fmt" | "certify" | "check-plan" | "prove",
//!               "implies": ["gdpr"], "baseline": "old.bwk",
//!               "main": "app.bwk", "files": { "app.bwk": "...", ... },
//!               "policies": ["gdpr", "org.bwk"], "catalog": "offers.bwk" | null,
//!               "plans": ["plan.json"], "vendor": "aws" | null,
//!               "place": ["api=offer"], "format": "text" | "json" | "markdown" }`
//!   Result: `{ "exit": 0 | 1 | 2, "stdout": "...", "stderr": "..." }`

use std::path::Path;

use bailiwick::cert;
use bailiwick::diag::{Diag, Severity};
use bailiwick::json::{self, Json};
use bailiwick::{builtin, loader, plancheck, prove, report};

static mut RESULT: Vec<u8> = Vec::new();

/// Memory for the host to write input into.
#[no_mangle]
pub extern "C" fn bwk_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Give back memory from `bwk_alloc`.
///
/// # Safety
/// `ptr` and `len` must come from one `bwk_alloc` call, freed once.
#[no_mangle]
pub unsafe extern "C" fn bwk_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len));
}

unsafe fn input(ptr: *const u8, len: usize) -> String {
    String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned()
}

unsafe fn output(j: &Json) -> *const u8 {
    let r = &mut *std::ptr::addr_of_mut!(RESULT);
    *r = json::to_string_compact(j).into_bytes();
    r.as_ptr()
}

#[no_mangle]
pub extern "C" fn bwk_result_len() -> usize {
    unsafe { (*std::ptr::addr_of!(RESULT)).len() }
}

/// The playground: evaluate one program.
///
/// # Safety
/// `ptr`/`len` must be a readable region written by the caller.
#[no_mangle]
pub unsafe extern "C" fn bwk_run(ptr: *const u8, len: usize) -> *const u8 {
    output(&run(&input(ptr, len)))
}

/// A command, as the native CLI would run it.
///
/// # Safety
/// `ptr`/`len` must be a readable region written by the caller.
#[no_mangle]
pub unsafe extern "C" fn bwk_call(ptr: *const u8, len: usize) -> *const u8 {
    let out = match json::parse(&input(ptr, len)) {
        Ok(req) => call(&req),
        Err(e) => Out::usage(format!("bailiwick: the request is not JSON: {}", e)),
    };
    output(&Json::obj(vec![
        ("exit", Json::Num(out.exit as f64)),
        ("stdout", Json::str(out.stdout)),
        ("stderr", Json::str(out.stderr)),
    ]))
}

fn diag_json(d: &Diag) -> Json {
    Json::obj(vec![
        ("line", Json::Num(d.line as f64)),
        ("col", Json::Num(d.col as f64)),
        ("severity", Json::str(if d.severity == Severity::Error { "error" } else { "warning" })),
        ("message", Json::str(&d.msg)),
    ])
}

/// No files to read in the browser: only packs resolve.
fn no_files(p: &Path) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("`{}`: the playground has no files; use a rule pack such as `import \"gdpr\"`", p.display()),
    ))
}

pub fn run(src: &str) -> Json {
    let loaded = loader::load_source_with(Path::new("playground.bwk"), src, Vec::new(), &no_files);
    let file = loaded.file;
    let diags = loaded.diags;
    let (formatted, _) = bailiwick::format_src(src);
    let own = !file.offers.is_empty();
    let offers = if own { file.offers.clone() } else { builtin::example_offers() };
    let has_errors = bailiwick::any_errors(&diags);
    let (text, plan, certificate) = if has_errors || file.workloads.is_empty() {
        (String::new(), Json::Null, Json::Null)
    } else {
        let plans = bailiwick::plan_file(&file, &offers);
        let certificate = cert::certify(&file, &offers, &[]).unwrap_or(Json::Null);
        (report::text(&plans), report::json(&plans), certificate)
    };
    Json::obj(vec![
        ("diagnostics", Json::Arr(diags.iter().map(diag_json).collect())),
        ("formatted", Json::str(formatted)),
        ("report", Json::str(text)),
        ("plan", plan),
        ("certificate", certificate),
        ("catalog", Json::str(if own { "own" } else { "example" })),
        ("offers", Json::Num(offers.len() as f64)),
    ])
}

struct Out {
    exit: u8,
    stdout: String,
    stderr: String,
}

impl Out {
    fn ok(stdout: String) -> Out {
        Out { exit: 0, stdout, stderr: String::new() }
    }
    fn fail(stderr: String) -> Out {
        Out { exit: 1, stdout: String::new(), stderr }
    }
    fn usage(stderr: String) -> Out {
        Out { exit: 2, stdout: String::new(), stderr }
    }
}

fn strs(j: Option<&Json>) -> Vec<String> {
    j.and_then(|a| a.as_arr()).unwrap_or(&[]).iter().filter_map(|x| x.as_str().map(String::from)).collect()
}

fn diags_text(path: &str, diags: &[Diag]) -> String {
    diags.iter().map(|d| format!("{}:{}\n", path, d)).collect()
}

fn call(req: &Json) -> Out {
    let command = req.get("command").and_then(|c| c.as_str()).unwrap_or("");
    let Some(main) = req.get("main").and_then(|m| m.as_str()) else {
        return Out::usage("bailiwick: expected a file".into());
    };
    let files: Vec<(String, String)> = match req.get("files") {
        Some(Json::Obj(kv)) => kv.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
        _ => Vec::new(),
    };
    let read = |p: &Path| -> std::io::Result<String> {
        let key = p.to_string_lossy();
        files
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))
    };
    let Ok(src) = read(Path::new(main)) else {
        return Out::usage(format!("bailiwick: cannot read `{}`", main));
    };
    let format = req.get("format").and_then(|f| f.as_str()).unwrap_or("text");

    if command == "fmt" {
        let (out, diags) = bailiwick::format_src(&src);
        return if bailiwick::any_errors(&diags) {
            Out::fail(format!("{}bailiwick: not formatting a file with errors\n", diags_text(main, &diags)))
        } else {
            Out::ok(out)
        };
    }

    // Policies: rule packs by name, or files the host supplied.
    let mut extra = Vec::new();
    for p in strs(req.get("policies")) {
        let loaded = match builtin::pack(&p) {
            Some(pack) if builtin::is_pack_name(&p) => loader::load_source_with(Path::new(&p), pack, Vec::new(), &read),
            _ => match read(Path::new(&p)) {
                Ok(s) => loader::load_source_with(Path::new(&p), &s, Vec::new(), &read),
                Err(_) => return Out::usage(format!("bailiwick: cannot read policy `{}`", p)),
            },
        };
        if bailiwick::any_errors(&loaded.diags) {
            return Out::fail(diags_text(&p, &loaded.diags));
        }
        extra.extend(loaded.file.policies);
    }

    let loaded = loader::load_source_with(Path::new(main), &src, extra, &read);
    let mut stderr = diags_text(main, &loaded.diags);
    if bailiwick::any_errors(&loaded.diags) {
        stderr.push_str(&format!("bailiwick: {} error(s)\n", loaded.diags.iter().filter(|d| d.is_error()).count()));
        return Out::fail(stderr);
    }
    let file = loaded.file;
    if command == "check" {
        return Out { exit: 0, stdout: format!("ok: {}\n", main), stderr };
    }

    let offers = match req.get("catalog").and_then(|c| c.as_str()) {
        // `check-plan` and `prove` need no offers: they speak about plans,
        // and about every possible offer.
        None if file.offers.is_empty() && (command == "check-plan" || command == "prove") => Vec::new(),
        None if file.offers.is_empty() => {
            return Out::usage(format!(
                "bailiwick {}: no offers: pass --catalog <offers.bwk>, or declare `offer` blocks\n",
                command
            ))
        }
        None => file.offers.clone(),
        Some(c) => match read(Path::new(c)) {
            Ok(s) => {
                let cat = loader::load_source_with(Path::new(c), &s, Vec::new(), &read);
                if bailiwick::any_errors(&cat.diags) {
                    return Out::fail(diags_text(c, &cat.diags));
                }
                cat.file.offers
            }
            Err(_) => return Out::usage(format!("bailiwick: cannot read catalog `{}`", c)),
        },
    };

    match command {
        "explain" => {
            let plans = bailiwick::plan_file(&file, &offers);
            let blocked = plans.iter().any(|p| !p.blocking().is_empty());
            let stdout = match format {
                "json" => report::json_string(&plans),
                "markdown" => report::markdown(&plans),
                _ => report::text(&plans),
            };
            Out { exit: u8::from(blocked), stdout, stderr }
        }
        "certify" => {
            let mut placements = Vec::new();
            for p in strs(req.get("place")) {
                match cert::Placement::parse(&p) {
                    Ok(p) => placements.push(p),
                    Err(e) => return Out::usage(format!("bailiwick certify: {}", e)),
                }
            }
            match cert::certify(&file, &offers, &placements) {
                Ok(c) => {
                    let deployable = c.get("deployable").and_then(|d| d.as_bool()) == Some(true);
                    Out { exit: u8::from(!deployable), stdout: json::to_string_pretty(&c) + "\n", stderr }
                }
                Err(e) => Out::fail(format!("bailiwick certify: {}\n", e)),
            }
        }
        "check-plan" => {
            let vendor = req.get("vendor").and_then(|v| v.as_str());
            let mut resources = Vec::new();
            for p in strs(req.get("plans")) {
                let Ok(text) = read(Path::new(&p)) else {
                    return Out::usage(format!("bailiwick: cannot read `{}`", p));
                };
                let got = if text.trim_start().starts_with('{') {
                    plancheck::terraform_resources(&text)
                } else if let Some(v) = vendor {
                    plancheck::kubernetes_resources(&text, v)
                } else {
                    return Out::usage(format!(
                        "bailiwick check-plan: `{}` looks like Kubernetes manifests; say whose cluster with --vendor",
                        p
                    ));
                };
                match got {
                    Ok(r) => resources.extend(r),
                    Err(e) => return Out::fail(format!("bailiwick check-plan: {}: {}\n", p, e)),
                }
            }
            let checks = plancheck::check(&file, &resources);
            let (text, ok) = plancheck::report(&checks);
            let stdout = if format == "markdown" { plancheck::markdown(&checks) } else { text };
            Out { exit: u8::from(!ok), stdout, stderr }
        }
        "prove" => {
            let implies = strs(req.get("implies"));
            let mut extra = Vec::new();
            for p in &implies {
                let src = match builtin::pack(p) {
                    Some(s) if builtin::is_pack_name(p) => s.to_string(),
                    _ => match read(Path::new(p)) {
                        Ok(s) => s,
                        Err(_) => return Out::usage(format!("bailiwick: cannot read policy `{}`", p)),
                    },
                };
                extra.extend(loader::load_source_with(Path::new(p), &src, Vec::new(), &read).file.policies);
            }
            let baseline = match req.get("baseline").and_then(|b| b.as_str()) {
                None => None,
                Some(b) => match read(Path::new(b)) {
                    Ok(s) => {
                        let l = loader::load_source_with(Path::new(b), &s, Vec::new(), &read);
                        if bailiwick::any_errors(&l.diags) {
                            return Out::fail(diags_text(b, &l.diags));
                        }
                        Some(l.file)
                    }
                    Err(_) => return Out::usage(format!("bailiwick: cannot read `{}`", b)),
                },
            };
            let mut all = true;
            let mut stdout = String::new();
            for w in &file.workloads {
                let answers = match &baseline {
                    Some(old) => match old.workloads.iter().find(|o| o.name == w.name) {
                        Some(ow) => prove::no_looser(w, &file.policies, ow, &old.policies, "allows nothing the earlier version did not"),
                        None => continue,
                    },
                    None if !extra.is_empty() => prove::implies(w, &file.policies, &extra),
                    None => prove::satisfiable(w, &file.policies),
                };
                let (text, ok) = prove::report(&answers);
                all &= ok;
                stdout.push_str(&format!("workload {}  (for every offer any provider could make)\n{}", w.name, text));
            }
            Out { exit: u8::from(!all), stdout, stderr }
        }
        other => Out::usage(format!("bailiwick: unknown command `{}`", other)),
    }
}
