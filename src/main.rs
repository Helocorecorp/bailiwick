//! `bailiwick` command-line interface.
//!
//!   bailiwick init    [dir] [--force]
//!   bailiwick check   <file.bwk> [--policy <p.bwk>]...
//!   bailiwick fmt     <file.bwk> [--write]
//!   bailiwick explain <file.bwk> --catalog <offers.bwk> [--policy <p.bwk>]... [--json | --proto]
//!   bailiwick ir      <file.bwk> [--policy <p.bwk>]... [--proto]
//!   bailiwick import  fly|compose|k8s|terraform <path>... [--out <file.bwk>]
//!   bailiwick prove   <file.bwk> [--implies <p>]... [--no-looser-than <old.bwk>] [--policy <p.bwk>]...
//!   bailiwick check-plan <file.bwk> <plan.json | manifests.yaml...> [--vendor <v>] [--policy <p.bwk>]...
//!   bailiwick certify <file.bwk> --catalog <offers.bwk> [--policy <p.bwk>]... [--place <unit=offer,...>]...
//!   bailiwick verify  <cert.json> <file.bwk> --catalog <offers.bwk> [--policy <p.bwk>]...
//!   bailiwick proto   decode <file.pb>
//!   bailiwick lsp
//!
//! Exit codes: 0 on success, 1 on diagnostics/blocked plans, 2 on usage.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use bailiwick::diag::{any_errors, Diag};
use bailiwick::feasibility::plan_file;
use bailiwick::{cert, import, ir, loader, proto, report};

struct Args {
    positional: Vec<String>,
    policies: Vec<String>,
    catalog: Option<String>,
    place: Vec<String>,
    vendor: Option<String>,
    implies: Vec<String>,
    baseline: Option<String>,
    out: Option<String>,
    json: bool,
    markdown: bool,
    proto: bool,
    write: bool,
    force: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        positional: Vec::new(),
        policies: Vec::new(),
        catalog: None,
        place: Vec::new(),
        vendor: None,
        implies: Vec::new(),
        baseline: None,
        out: None,
        json: false,
        markdown: false,
        proto: false,
        write: false,
        force: false,
    };
    let mut i = 0;
    while i < args.len() {
        let s = args[i].as_str();
        match s {
            "--policy" | "--catalog" | "--out" | "--place" | "--vendor" | "--implies" | "--no-looser-than" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| format!("{} needs a path", s))?.clone();
                match s {
                    "--policy" => a.policies.push(v),
                    "--catalog" => a.catalog = Some(v),
                    "--place" => a.place.push(v),
                    "--vendor" => a.vendor = Some(v),
                    "--implies" => a.implies.push(v),
                    "--no-looser-than" => a.baseline = Some(v),
                    _ => a.out = Some(v),
                }
            }
            "--json" => a.json = true,
            "--markdown" => a.markdown = true,
            "--proto" => a.proto = true,
            "--write" => a.write = true,
            "--force" => a.force = true,
            s if s.starts_with("--") => return Err(format!("unknown flag `{}`", s)),
            _ => a.positional.push(s.to_string()),
        }
        i += 1;
    }
    Ok(a)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        return usage();
    }
    let cmd = argv[1].as_str();
    if matches!(cmd, "help" | "--help" | "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if cmd == "lsp" {
        let code = bailiwick::lsp::run_stdio();
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    let args = match parse_args(&argv[2..]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bailiwick {}: {}", cmd, e);
            return ExitCode::from(2);
        }
    };
    match cmd {
        "init" => cmd_init(&args),
        "check" => cmd_check(&args),
        "fmt" => cmd_fmt(&args),
        "explain" => cmd_explain(&args),
        "certify" => cmd_certify(&args),
        "verify" => cmd_verify(&args),
        "check-plan" => cmd_check_plan(&args),
        "prove" => cmd_prove(&args),
        "ir" => cmd_ir(&args),
        "import" => cmd_import(&args),
        "proto" => cmd_proto(&args),
        other => {
            eprintln!("bailiwick: unknown command `{}`", other);
            usage()
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: bailiwick <init|check|fmt|explain|prove|check-plan|certify|verify|ir|import|proto|lsp|help> ...");
    eprintln!("try `bailiwick help`");
    ExitCode::from(2)
}

fn print_help() {
    println!("bailiwick — reference toolchain for the Bailiwick");
    println!();
    println!("commands:");
    println!("  init    [dir] [--force]                         write bailiwick.bwk from the project's own files");
    println!("  check   <file> [--policy <p>]...               parse, type-check, resolve imports");
    println!("  fmt     <file> [--write]                        canonical formatting");
    println!("  explain <file> [--catalog <offers>]             per-service feasibility report");
    println!("                 [--policy <p>]... [--json|--proto|--markdown]  (JSON/proto = the plan IR)");
    println!("  prove   <file> [--policy <p>]...               can each part run anywhere at all, for any provider?");
    println!("          <file> --implies <p>...                 does everything it allows already satisfy <p>?");
    println!("          <file> --no-looser-than <old.bwk>      is this version no looser than <old>, for any provider?");
    println!("  check-plan <file> <plan.json>                  hold a Terraform plan (`terraform show -json`) to the rules");
    println!("  check-plan <file> <k8s.yaml>... --vendor <v>   ... or Kubernetes manifests on a cluster at aws|gcp|azure|...");
    println!("  certify <file> --catalog <offers>               deployment certificate (JSON), checkable offline");
    println!("                 [--policy <p>]... [--place <unit=offer,...>]...");
    println!("  verify  <cert> <file> --catalog <offers>        check a certificate against its sources");
    println!("                 [--policy <p>]...                  (and with `bwk-model verify` against the proofs)");
    println!("  ir      <file> [--policy <p>]... [--proto]     canonical IR (bwk/0) as JSON or protobuf");
    println!("  import  fly <fly.toml> [--out <f.bwk>]          fly.toml → BWK skeleton");
    println!("  import  compose <docker-compose.yml> [--out]    compose → BWK skeleton");
    println!("  import  k8s <manifests.yaml> [--out]            Kubernetes manifests → BWK skeleton");
    println!("  import  terraform <main.tf> [more.tf...] [--out] Terraform (HCL) → BWK skeleton");
    println!("  proto   decode <file.pb>                        protobuf IR → JSON IR");
    println!("  lsp                                             language server over stdio");
    println!();
    println!("--policy files are composed with any `import`ed policies; policies only narrow.");
    println!("--policy and `import` also take a built-in rule pack by name, e.g. `import \"gdpr\"`:");
    println!("  {}", bailiwick::builtin::pack_names());
    println!("Without --catalog, the program's own `offer` blocks are used.");
}

fn print_diags(path: &str, diags: &[Diag]) {
    for d in diags {
        eprintln!("{}:{}", path, d);
    }
}

fn need_file<'a>(args: &'a Args, cmd: &str) -> Result<&'a str, ExitCode> {
    match args.positional.first() {
        Some(p) => Ok(p.as_str()),
        None => {
            eprintln!("bailiwick {}: expected a file", cmd);
            Err(ExitCode::from(2))
        }
    }
}

fn write_bytes(bytes: &[u8]) -> ExitCode {
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(bytes).and_then(|_| out.flush()) {
        eprintln!("bailiwick: cannot write output: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Load the main file with imports resolved and any --policy files merged.
fn load_with_policies(path: &str, policy_paths: &[String]) -> Result<bailiwick::File, ExitCode> {
    let (extra, pdiags) = loader::load_policies(policy_paths);
    for d in &pdiags {
        eprintln!("{}", d);
    }
    if any_errors(&pdiags) {
        return Err(ExitCode::FAILURE);
    }
    let loaded = loader::load(Path::new(path), extra);
    print_diags(path, &loaded.diags);
    if any_errors(&loaded.diags) {
        eprintln!(
            "bailiwick: {} error(s)",
            loaded.diags.iter().filter(|d| d.is_error()).count()
        );
        return Err(ExitCode::FAILURE);
    }
    Ok(loaded.file)
}

fn cmd_init(args: &Args) -> ExitCode {
    let dir = args.positional.first().map(|s| s.as_str()).unwrap_or(".");
    let out = Path::new(dir).join("bailiwick.bwk");
    if out.exists() && !args.force {
        eprintln!("bailiwick init: `{}` already exists (use --force to replace it)", out.display());
        return ExitCode::FAILURE;
    }
    let init = match bailiwick::init::init(Path::new(dir)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("bailiwick init: {}", e);
            return ExitCode::from(2);
        }
    };
    // Every importer promises valid BWK; check the whole before writing.
    let (_f, diags) = bailiwick::parse_and_check(&init.bwk);
    if any_errors(&diags) {
        eprintln!("bailiwick init: internal error: the generated file does not check:");
        print_diags("<generated>", &diags);
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&out, &init.bwk) {
        eprintln!("bailiwick: cannot write `{}`: {}", out.display(), e);
        return ExitCode::from(2);
    }
    if init.sources.is_empty() {
        println!("wrote {}: a starting point (no fly.toml, Compose, Kubernetes or Terraform files found)", out.display());
    } else {
        println!("wrote {} from:", out.display());
        for s in &init.sources {
            println!("  {}", s);
        }
    }
    for n in &init.notes {
        println!("note: {}", n);
    }
    println!("next: confirm each `TODO`, uncomment the rule packs that apply, then run");
    println!("  bailiwick explain {}", out.display());
    ExitCode::SUCCESS
}

fn cmd_check(args: &Args) -> ExitCode {
    let path = match need_file(args, "check") {
        Ok(p) => p,
        Err(c) => return c,
    };
    match load_with_policies(path, &args.policies) {
        Ok(_) => {
            println!("ok: {}", path);
            ExitCode::SUCCESS
        }
        Err(c) => c,
    }
}

fn cmd_fmt(args: &Args) -> ExitCode {
    let path = match need_file(args, "fmt") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bailiwick: cannot read `{}`: {}", path, e);
            return ExitCode::from(2);
        }
    };
    let (out, diags) = bailiwick::format_src(&src);
    if any_errors(&diags) {
        print_diags(path, &diags);
        eprintln!("bailiwick: not formatting a file with errors");
        return ExitCode::FAILURE;
    }
    if args.write {
        if let Err(e) = std::fs::write(path, &out) {
            eprintln!("bailiwick: cannot write `{}`: {}", path, e);
            return ExitCode::from(2);
        }
        println!("formatted: {}", path);
    } else {
        print!("{}", out);
    }
    ExitCode::SUCCESS
}

/// Load a program and its catalog of offers.
fn load_program(cmd: &str, path: &str, args: &Args) -> Result<(bailiwick::File, Vec<bailiwick::Offer>), ExitCode> {
    let file = load_with_policies(path, &args.policies)?;
    let catalog_path = match &args.catalog {
        Some(c) => c.clone(),
        // No catalog given: the offers the program itself declares.
        None => {
            if file.offers.is_empty() {
                eprintln!(
                    "bailiwick {}: no offers: pass --catalog <offers.bwk>, or declare `offer` blocks (fictional examples: examples/catalog/all.bwk)",
                    cmd
                );
                return Err(ExitCode::from(2));
            }
            let offers = file.offers.clone();
            return Ok((file, offers));
        }
    };
    let catalog = loader::load(Path::new(&catalog_path), Vec::new());
    print_diags(&catalog_path, &catalog.diags);
    if any_errors(&catalog.diags) {
        return Err(ExitCode::FAILURE);
    }
    if catalog.file.offers.is_empty() {
        eprintln!("bailiwick {}: catalog `{}` defines no offers", cmd, catalog_path);
        return Err(ExitCode::FAILURE);
    }
    Ok((file, catalog.file.offers))
}

fn cmd_certify(args: &Args) -> ExitCode {
    let path = match need_file(args, "certify") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let (file, offers) = match load_program("certify", path, args) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let mut placements = Vec::new();
    for p in &args.place {
        match cert::Placement::parse(p) {
            Ok(p) => placements.push(p),
            Err(e) => {
                eprintln!("bailiwick certify: {}", e);
                return ExitCode::from(2);
            }
        }
    }
    match cert::certify(&file, &offers, &placements) {
        Ok(c) => {
            println!("{}", bailiwick::json::to_string_pretty(&c));
            if c.get("deployable").and_then(|d| d.as_bool()) == Some(true) {
                ExitCode::SUCCESS
            } else {
                eprintln!("bailiwick certify: certified, but a unit fails closed (see `deploys`)");
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("bailiwick certify: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_prove(args: &Args) -> ExitCode {
    let path = match need_file(args, "prove") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let file = match load_with_policies(path, &args.policies) {
        Ok(f) => f,
        Err(c) => return c,
    };
    let extra = if args.implies.is_empty() {
        Vec::new()
    } else {
        let (ps, diags) = loader::load_policies(&args.implies);
        for d in &diags {
            eprintln!("{}", d);
        }
        if any_errors(&diags) {
            return ExitCode::FAILURE;
        }
        ps
    };
    let old = match &args.baseline {
        Some(b) => match load_with_policies(b, &args.policies) {
            Ok(f) => Some(f),
            Err(c) => return c,
        },
        None => None,
    };
    let mut all = true;
    for w in &file.workloads {
        let answers = if let Some(old) = &old {
            match old.workloads.iter().find(|o| o.name == w.name) {
                Some(ow) => bailiwick::prove::no_looser(
                    w,
                    &file.policies,
                    ow,
                    &old.policies,
                    "allows nothing the earlier version did not",
                ),
                None => {
                    println!("workload {}: new (not in the earlier version)", w.name);
                    continue;
                }
            }
        } else if !extra.is_empty() {
            bailiwick::prove::implies(w, &file.policies, &extra)
        } else {
            bailiwick::prove::satisfiable(w, &file.policies)
        };
        let (text, ok) = bailiwick::prove::report(&answers);
        all &= ok;
        println!("workload {}  (for every offer any provider could make)", w.name);
        print!("{}", text);
    }
    if all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn cmd_check_plan(args: &Args) -> ExitCode {
    let (path, plans) = match args.positional.split_first() {
        Some((p, rest)) if !rest.is_empty() => (p.as_str(), rest),
        _ => {
            eprintln!("bailiwick check-plan: expected <file.bwk> <plan.json | manifests.yaml...>");
            return ExitCode::from(2);
        }
    };
    let file = match load_with_policies(path, &args.policies) {
        Ok(f) => f,
        Err(c) => return c,
    };
    let mut resources = Vec::new();
    for p in plans {
        let text = match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("bailiwick: cannot read `{}`: {}", p, e);
                return ExitCode::from(2);
            }
        };
        let got = if text.trim_start().starts_with('{') {
            bailiwick::plancheck::terraform_resources(&text)
        } else {
            match &args.vendor {
                Some(v) => bailiwick::plancheck::kubernetes_resources(&text, v),
                None => {
                    eprintln!("bailiwick check-plan: `{}` looks like Kubernetes manifests; say whose cluster with --vendor aws|gcp|azure|digitalocean|hetzner", p);
                    return ExitCode::from(2);
                }
            }
        };
        match got {
            Ok(r) => resources.extend(r),
            Err(e) => {
                eprintln!("bailiwick check-plan: {}: {}", p, e);
                return ExitCode::FAILURE;
            }
        }
    }
    let checks = bailiwick::plancheck::check(&file, &resources);
    let (text, ok) = bailiwick::plancheck::report(&checks);
    if args.markdown {
        print!("{}", bailiwick::plancheck::markdown(&checks));
    } else {
        print!("{}", text);
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn cmd_verify(args: &Args) -> ExitCode {
    let (cert_path, path) = match (args.positional.first(), args.positional.get(1)) {
        (Some(c), Some(p)) => (c.as_str(), p.as_str()),
        _ => {
            eprintln!("bailiwick verify: expected <cert.json> <file.bwk>");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(cert_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("bailiwick: cannot read `{}`: {}", cert_path, e);
            return ExitCode::from(2);
        }
    };
    let c = match bailiwick::json::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bailiwick verify: `{}` is not JSON: {}", cert_path, e);
            return ExitCode::FAILURE;
        }
    };
    let (file, offers) = match load_program("verify", path, args) {
        Ok(x) => x,
        Err(c) => return c,
    };
    match cert::verify(&c, &file, &offers) {
        Ok(()) => {
            println!("ok: `{}` is exactly what these inputs produce", cert_path);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bailiwick verify: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_explain(args: &Args) -> ExitCode {
    let path = match need_file(args, "explain") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let (file, offers) = match load_program("explain", path, args) {
        Ok(x) => x,
        Err(c) => return c,
    };

    let plans = plan_file(&file, &offers);
    let blocked = plans.iter().any(|p| !p.blocking().is_empty());
    if args.proto {
        let code = write_bytes(&proto::encode_plans(&plans));
        if code != ExitCode::SUCCESS {
            return code;
        }
    } else if args.json {
        print!("{}", report::json_string(&plans));
    } else if args.markdown {
        print!("{}", report::markdown(&plans));
    } else {
        print!("{}", report::text(&plans));
    }
    if blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_ir(args: &Args) -> ExitCode {
    let path = match need_file(args, "ir") {
        Ok(p) => p,
        Err(c) => return c,
    };
    match load_with_policies(path, &args.policies) {
        Ok(file) => {
            if args.proto {
                write_bytes(&proto::encode_file(&file))
            } else {
                print!("{}", ir::to_string(&file));
                ExitCode::SUCCESS
            }
        }
        Err(c) => c,
    }
}

fn cmd_proto(args: &Args) -> ExitCode {
    match args.positional.first().map(|s| s.as_str()) {
        Some("decode") => {}
        _ => {
            eprintln!("bailiwick proto: expected `decode <file.pb>`");
            return ExitCode::from(2);
        }
    }
    let path = match args.positional.get(1) {
        Some(p) => p,
        None => {
            eprintln!("bailiwick proto decode: expected a file");
            return ExitCode::from(2);
        }
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("bailiwick: cannot read `{}`: {}", path, e);
            return ExitCode::from(2);
        }
    };
    match proto::decode_file(&bytes) {
        Ok(file) => {
            print!("{}", ir::to_string(&file));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bailiwick proto decode: {}: {}", path, e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_import(args: &Args) -> ExitCode {
    let kind = match args.positional.first() {
        Some(k) => k.as_str(),
        None => {
            eprintln!("bailiwick import: expected `fly`, `compose`, `k8s` or `terraform`");
            return ExitCode::from(2);
        }
    };
    let paths = &args.positional[1..];
    if paths.is_empty() {
        eprintln!("bailiwick import {}: expected a file", kind);
        return ExitCode::from(2);
    }
    // Terraform modules span several files; the others take one.
    if paths.len() > 1 && !matches!(kind, "terraform" | "tf") {
        eprintln!("bailiwick import {}: expected exactly one file", kind);
        return ExitCode::from(2);
    }
    let mut src = String::new();
    for p in paths {
        match std::fs::read_to_string(p) {
            Ok(s) => {
                src.push_str(&s);
                src.push('\n');
            }
            Err(e) => {
                eprintln!("bailiwick: cannot read `{}`: {}", p, e);
                return ExitCode::from(2);
            }
        }
    }
    let path = paths.join(", ");
    let result = match kind {
        "fly" => import::fly::import(&src),
        "compose" => import::compose::import(&src),
        "k8s" | "kubernetes" => import::k8s::import(&src),
        "terraform" | "tf" => import::terraform::import(&src),
        other => {
            eprintln!(
                "bailiwick import: unknown source `{}` (use fly, compose, k8s, or terraform)",
                other
            );
            return ExitCode::from(2);
        }
    };
    let imported = match result {
        Ok(i) => i,
        Err(e) => {
            eprintln!("bailiwick import {}: {}: {}", kind, path, e);
            return ExitCode::FAILURE;
        }
    };
    for n in &imported.notes {
        eprintln!("note: {}", n);
    }
    // The importer promises valid BWK; verify rather than trust.
    let (_f, diags) = bailiwick::parse_and_check(&imported.bwk);
    if any_errors(&diags) {
        eprintln!("bailiwick import: internal error — generated BWK does not check:");
        print_diags("<generated>", &diags);
        return ExitCode::FAILURE;
    }
    match &args.out {
        Some(out) => {
            if let Err(e) = std::fs::write(out, &imported.bwk) {
                eprintln!("bailiwick: cannot write `{}`: {}", out, e);
                return ExitCode::from(2);
            }
            println!("wrote: {}", out);
        }
        None => print!("{}", imported.bwk),
    }
    ExitCode::SUCCESS
}
