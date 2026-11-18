//! `bailiwick init` finds a project's deployment files and writes a file
//! that checks.

use std::fs;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bwk-init-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn example(f: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/import").join(f)).unwrap()
}

#[test]
fn it_imports_every_kind_of_deployment_file_it_finds() {
    let d = scratch("all");
    fs::create_dir_all(d.join("infra")).unwrap();
    fs::create_dir_all(d.join("deploy")).unwrap();
    fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
    fs::write(d.join("fly.toml"), example("fly.toml")).unwrap();
    fs::write(d.join("compose.yaml"), example("docker-compose.yml")).unwrap();
    fs::write(d.join("infra/main.tf"), example("main.tf")).unwrap();
    fs::write(d.join("deploy/app.yaml"), example("k8s.yaml")).unwrap();
    fs::write(d.join("node_modules/pkg/fly.toml"), example("fly.toml")).unwrap();
    fs::write(d.join("deploy/values.yaml"), "replicas: 3\n").unwrap();

    let init = bailiwick::init::init(&d).unwrap();
    assert_eq!(init.sources, vec!["compose.yaml", "fly.toml", "infra/main.tf", "deploy/app.yaml"]);
    let (file, diags) = bailiwick::parse_and_check(&init.bwk);
    assert!(!bailiwick::any_errors(&diags), "{:?}\n{}", diags, init.bwk);
    assert_eq!(file.workloads.len(), 4);
    let mut names: Vec<_> = file.workloads.iter().map(|w| w.name.clone()).collect();
    names.dedup();
    assert_eq!(names.len(), 4, "workload names are distinct");
    assert!(init.bwk.contains("// import \"gdpr\""));
}

#[test]
fn two_terraform_directories_get_distinct_workloads() {
    let d = scratch("tf2");
    for sub in ["a", "b"] {
        fs::create_dir_all(d.join(sub)).unwrap();
        fs::write(d.join(sub).join("main.tf"), example("main.tf")).unwrap();
    }
    let init = bailiwick::init::init(&d).unwrap();
    let (file, diags) = bailiwick::parse_and_check(&init.bwk);
    assert!(!bailiwick::any_errors(&diags), "{:?}", diags);
    assert_eq!(file.workloads.len(), 2);
    assert_ne!(file.workloads[0].name, file.workloads[1].name);
}

#[test]
fn an_empty_project_gets_a_starting_point_that_deploys() {
    let d = scratch("empty");
    let init = bailiwick::init::init(&d).unwrap();
    assert!(init.sources.is_empty());
    let (file, diags) = bailiwick::parse_and_check(&init.bwk);
    assert!(diags.is_empty(), "{:?}", diags);
    let plans = bailiwick::plan_file(&file, &bailiwick::builtin::example_offers());
    assert!(plans[0].blocking().is_empty());
}
