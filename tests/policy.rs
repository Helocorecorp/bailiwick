//! Policies only narrow. These tests pin design rule 3: composing a policy
//! with a workload can remove candidates, never add them, and the result
//! does not depend on the order policies are listed.

use std::path::Path;

use bailiwick::feasibility::{plan_workload, Exclusion};
use bailiwick::{any_errors, parse_and_check, parse_offers};

fn offers() -> Vec<bailiwick::Offer> {
    let (offers, diags) = parse_offers(include_str!("../examples/catalog/core.bwk"));
    assert!(!any_errors(&diags));
    offers
}

#[test]
fn policy_residency_narrows_a_global_workload() {
    let src = r#"
        policy eu-only { residency = eu }
        workload w {
          service api : backend-api { residency = global  sensitivity = S0 }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &file.policies);
    let u = &plan.units[0];
    assert_eq!(u.effective_residency, "eu", "global ∩ eu must be eu");
    assert!(!u.feasible_ids().contains(&"e-us-va"));
    assert!(u.feasible_ids().contains(&"a-de"));
    assert_eq!(plan.policies, vec!["eu-only".to_string()]);
}

#[test]
fn workload_cannot_widen_past_policy() {
    // A workload asking for `us` under an `eu` policy is a conflict,
    // reported visibly at check time — not silently empty at plan time.
    let src = r#"
        policy eu-only { residency = eu }
        workload w {
          service api : backend-api { residency = us }
        }
    "#;
    let (_file, diags) = parse_and_check(src);
    assert!(
        diags.iter().any(|d| d.is_error() && d.msg.contains("conflicts with residency `eu`")),
        "{:?}",
        diags
    );
}

#[test]
fn policy_min_trust_raises_the_floor() {
    let src = r#"
        policy strict { min_trust = T3 }
        workload w {
          job batch : background-jobs { sensitivity = S0 }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &file.policies);
    let u = &plan.units[0];
    assert_eq!(u.effective_sensitivity, 0);
    assert_eq!(u.min_trust, 3, "S0 alone would be T1; the policy raises it to T3");
    assert!(!u.feasible_ids().contains(&"i-de"));
    assert!(!u.feasible_ids().contains(&"a-de"));
    assert!(u.feasible_ids().contains(&"b-fr"));
}

#[test]
fn policy_deny_vendor_excludes_with_attribution() {
    let src = r#"
        policy no-aws { deny vendors = [ vendor-e ] }
        workload w {
          service api : backend-api { }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &file.policies);
    let v = plan.units[0]
        .verdicts
        .iter()
        .find(|v| v.offer_id == "e-us-va")
        .unwrap();
    assert!(!v.feasible());
    assert!(v.exclusions.iter().any(|e| matches!(
        e,
        Exclusion::PolicyVendor { vendor, policy } if vendor == "vendor-e" && policy == "no-aws"
    )));
    assert!(v.exclusions[0].reason().contains("denied by policy `no-aws`"));
}

#[test]
fn policy_deny_country_excludes() {
    let src = r#"
        policy not-us { deny countries = [ us ] }
        workload w {
          service media : media-assets { }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &file.policies);
    let feas = plan.units[0].feasible_ids();
    assert!(!feas.contains(&"g-us"));
    assert!(!feas.contains(&"e-us-va"));
    assert!(feas.contains(&"a-de"));
}

#[test]
fn policies_compose_order_independently() {
    let a = "policy a { residency = eu }";
    let b = "policy b { min_trust = T3  deny vendors = [ vendor-c ] }";
    let w = r#"
        workload w {
          data d { sensitivity = S1 }
          service api : backend-api { depends_on = d }
          job batch : background-jobs { }
        }
    "#;
    let ab = format!("{}\n{}\n{}", a, b, w);
    let ba = format!("{}\n{}\n{}", b, a, w);
    let (fab, d1) = parse_and_check(&ab);
    let (fba, d2) = parse_and_check(&ba);
    assert!(!any_errors(&d1) && !any_errors(&d2));
    let pab = plan_workload(&fab.workloads[0], &offers(), &fab.policies);
    let pba = plan_workload(&fba.workloads[0], &offers(), &fba.policies);
    for (ua, ub) in pab.units.iter().zip(pba.units.iter()) {
        assert_eq!(ua.effective_residency, ub.effective_residency);
        assert_eq!(ua.min_trust, ub.min_trust);
        assert_eq!(ua.feasible_ids(), ub.feasible_ids());
    }
    // And the composition is strictly narrower than either policy alone.
    let only_a = plan_workload(&fab.workloads[0], &offers(), &fab.policies[..1]);
    let api_ab = pab.units[0].feasible_ids();
    let api_a = only_a.units[0].feasible_ids();
    assert!(api_ab.iter().all(|x| api_a.contains(x)), "composition must be a subset");
    assert!(api_ab.len() < api_a.len(), "b must have removed something");
}

#[test]
fn conflicting_policies_are_reported() {
    let src = r#"
        policy eu { residency = eu }
        policy us { residency = us }
        workload w { service api : backend-api { } }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(
        diags.iter().any(|d| d.is_error() && d.msg.contains("no overlap")),
        "{:?}",
        diags
    );
}

#[test]
fn budget_cap_is_enforced() {
    let src = r#"
        policy frugal { budget <= $10000 / month }
        workload w {
          budget = $40000 / month
          service api : backend-api { }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(
        diags.iter().any(|d| d.is_error() && d.msg.contains("exceeds")),
        "{:?}",
        diags
    );

    let ok = r#"
        policy frugal { budget <= $50000 / month }
        workload w {
          budget = $40000 / month
          service api : backend-api { }
        }
    "#;
    let (_f, diags) = parse_and_check(ok);
    assert!(!any_errors(&diags), "{:?}", diags);
}

#[test]
fn policy_budget_must_be_a_cap() {
    let src = "policy p { budget = $10 / month }";
    let (_f, diags) = parse_and_check(src);
    assert!(diags.iter().any(|d| d.msg.contains("cap")), "{:?}", diags);
}

#[test]
fn example_with_imported_policy_loads_and_narrows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loaded = bailiwick::loader::load(&root.join("examples/storefront-eu-policy.bwk"), Vec::new());
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    assert_eq!(loaded.file.policies.len(), 1, "the imported policy must be merged");
    assert_eq!(loaded.file.policies[0].name, "eu-only");
    let plans = bailiwick::plan_file(&loaded.file, &offers());
    let media = plans[0].units.iter().find(|u| u.name == "media").unwrap();
    assert_eq!(media.effective_residency, "eu", "media had no residency; policy supplies eu");
    assert!(!media.feasible_ids().contains(&"g-us"));
    assert!(plans[0].blocking().is_empty(), "{:?}", plans[0].blocking());
}

#[test]
fn loader_detects_import_cycles() {
    let dir = std::env::temp_dir().join(format!("bwk-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.bwk"), "import \"b.bwk\"\npolicy a { residency = eu }\n").unwrap();
    std::fs::write(dir.join("b.bwk"), "import \"a.bwk\"\npolicy b { min_trust = T2 }\n").unwrap();
    let loaded = bailiwick::loader::load(&dir.join("a.bwk"), Vec::new());
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    assert!(
        loaded.diags.iter().any(|d| d.msg.contains("cycle or duplicate")),
        "{:?}",
        loaded.diags
    );
    assert_eq!(loaded.file.policies.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_policy_files_apply_like_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (extra, d) = bailiwick::loader::load_policies(&[root
        .join("examples/policies/eu-only.bwk")
        .to_string_lossy()
        .to_string()]);
    assert!(!any_errors(&d), "{:?}", d);
    let loaded = bailiwick::loader::load(&root.join("examples/storefront.bwk"), extra);
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    let plans = bailiwick::plan_file(&loaded.file, &offers());
    assert_eq!(plans[0].policies, vec!["eu-only".to_string()]);
}
