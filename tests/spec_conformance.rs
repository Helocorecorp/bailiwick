//! Runs every case under `spec/conformance/cases/` against the frozen
//! catalog and compares the plan IR — both the JSON mapping and the
//! Protobuf encoding — byte-for-byte with the committed `*.expected.json`
//! and `*.expected.pb`. Any conforming implementation must produce the
//! same output; this is the suite other implementations run.
//!
//! To regenerate after an intentional spec change:
//!   BWK_UPDATE_EXPECTED=1 cargo test --test spec_conformance

use std::fs;
use std::path::{Path, PathBuf};

use bailiwick::{any_errors, loader, plan_file, proto, report};

fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/conformance")
}

#[test]
fn every_case_matches_its_expected_plan() {
    let dir = conformance_dir();
    let catalog = loader::load(&dir.join("catalog.bwk"), Vec::new());
    assert!(!any_errors(&catalog.diags), "catalog: {:?}", catalog.diags);

    let mut cases: Vec<PathBuf> = fs::read_dir(dir.join("cases"))
        .expect("spec/conformance/cases must exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "bwk").unwrap_or(false))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no conformance cases found");

    let update = std::env::var("BWK_UPDATE_EXPECTED").is_ok();
    let mut failures = Vec::new();

    for case in &cases {
        let loaded = loader::load(case, Vec::new());
        assert!(!any_errors(&loaded.diags), "{}: {:?}", case.display(), loaded.diags);
        let plans = plan_file(&loaded.file, &catalog.file.offers);
        let actual_json = report::json_string(&plans);
        let actual_pb = proto::encode_plans(&plans);
        let json_path = case.with_extension("expected.json");
        let pb_path = case.with_extension("expected.pb");
        if update {
            fs::write(&json_path, &actual_json).unwrap();
            fs::write(&pb_path, &actual_pb).unwrap();
            continue;
        }
        match fs::read_to_string(&json_path) {
            Ok(expected) => {
                if expected != actual_json {
                    failures.push(format!(
                        "{}: plan differs from {}\n--- expected\n{}\n--- actual\n{}",
                        case.display(),
                        json_path.display(),
                        expected,
                        actual_json
                    ));
                }
            }
            Err(_) => failures.push(format!(
                "{}: missing {} (run with BWK_UPDATE_EXPECTED=1 to create)",
                case.display(),
                json_path.display()
            )),
        }
        match fs::read(&pb_path) {
            Ok(expected) => {
                if expected != actual_pb {
                    failures.push(format!(
                        "{}: protobuf plan differs from {} ({} vs {} bytes)",
                        case.display(),
                        pb_path.display(),
                        expected.len(),
                        actual_pb.len()
                    ));
                }
            }
            Err(_) => failures.push(format!(
                "{}: missing {} (run with BWK_UPDATE_EXPECTED=1 to create)",
                case.display(),
                pb_path.display()
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
