//! Core conformance tests for the reference toolchain: unit typing, the
//! residency and trust filters, dependency propagation, fail-closed
//! redundancy, and formatting. See `tests/policy.rs` for policy
//! composition and `tests/spec_conformance.rs` for the frozen cases under
//! `spec/conformance/`.

use bailiwick::feasibility::plan_workload;
use bailiwick::{any_errors, parse_and_check, parse_offers};

fn offers() -> Vec<bailiwick::Offer> {
    let src = include_str!("../examples/catalog/core.bwk");
    let (offers, diags) = parse_offers(src);
    assert!(!any_errors(&diags), "catalog has errors: {:?}", diags);
    offers
}

#[test]
fn example_parses_clean() {
    let src = include_str!("../examples/storefront.bwk");
    let (_f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "example has errors: {:?}", diags);
}

#[test]
fn catalog_parses_clean() {
    let _ = offers();
}

#[test]
fn units_are_types_duration_rejects_money() {
    // slo.p95 expects a duration; money must be a type error.
    let src = r#"
        workload w {
          service s : backend-api { slo.p95 <= $5 }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(any_errors(&diags), "expected a unit error, got none");
    assert!(
        diags.iter().any(|d| d.msg.contains("duration")),
        "expected a duration-related message, got {:?}",
        diags
    );
}

#[test]
fn units_are_types_duration_requires_unit() {
    let src = r#"
        workload w {
          service s : backend-api { slo.p95 <= 200 }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(any_errors(&diags), "a bare number should need a unit");
}

#[test]
fn unknown_class_is_rejected() {
    let src = r#"
        workload w {
          service s : teleporter { }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(diags.iter().any(|d| d.msg.contains("unknown asset class")));
}

#[test]
fn unknown_region_is_rejected() {
    let src = r#"
        workload w {
          service s : backend-api { residency = atlantis }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(diags.iter().any(|d| d.msg.contains("unknown residency")));
}

#[test]
fn dangling_dependency_is_rejected() {
    let src = r#"
        workload w {
          service s : backend-api { depends_on = ghost }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(diags.iter().any(|d| d.msg.contains("depends_on")));
}

#[test]
fn service_and_data_with_disjoint_residency_is_rejected() {
    let src = r#"
        workload w {
          data d { residency = us }
          service s : backend-api { residency = eu  depends_on = d }
        }
    "#;
    let (_f, diags) = parse_and_check(src);
    assert!(
        diags.iter().any(|d| d.msg.contains("no region satisfies both")),
        "{:?}",
        diags
    );
}

#[test]
fn residency_filters_out_of_region_offers() {
    // A us-only data residency must exclude EU offers for a dependent service.
    let src = r#"
        workload w {
          data d { residency = us  sensitivity = S0 }
          service s : backend-api { depends_on = d }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &[]);
    let unit = &plan.units[0];
    assert_eq!(unit.effective_residency, "us");
    let feas = unit.feasible_ids();
    assert!(feas.contains(&"e-us-va"), "vendor-e should be feasible: {:?}", feas);
    assert!(
        !feas.contains(&"a-de"),
        "vendor-a (de) must be excluded under us residency"
    );
}

#[test]
fn sensitivity_propagates_from_dependency_and_raises_trust_floor() {
    // The service itself declares no sensitivity, but its S3 dependency
    // must push the trust floor to T3.
    let src = r#"
        workload w {
          data d { residency = eu  sensitivity = S3 }
          service s : backend-api { residency = eu  depends_on = d }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &[]);
    let unit = &plan.units[0];
    assert_eq!(unit.effective_sensitivity, 3);
    assert_eq!(unit.min_trust, 3);
    let v = unit
        .verdicts
        .iter()
        .find(|v| v.offer_id == "a-de")
        .unwrap();
    assert!(!v.feasible(), "vendor-a T2 must be excluded at S3");
    assert!(v.exclusions.iter().any(|e| matches!(
        e,
        bailiwick::feasibility::Exclusion::Trust { .. }
    )));
}

#[test]
fn redundancy_fails_closed_when_too_few_vendors() {
    let catalog = r#"
        offer only-eu-t3 { vendor = vendor-b  country = fr  trust = T3  classes = [ backend-api ] }
        offer eu-t2      { vendor = vendor-a   country = de  trust = T2  classes = [ backend-api ] }
    "#;
    let (offers, od) = parse_offers(catalog);
    assert!(!any_errors(&od));
    let src = r#"
        workload w {
          data d { residency = eu  sensitivity = S3 }
          service s : backend-api {
            residency = eu
            depends_on = d
            redundancy { vendors >= 2 }
          }
        }
    "#;
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers, &[]);
    assert_eq!(plan.units[0].distinct_feasible_vendors(), 1);
    assert!(!plan.blocking().is_empty(), "should be blocked on redundancy");
}

#[test]
fn storefront_is_deployable_against_catalog() {
    let src = include_str!("../examples/storefront.bwk");
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&file.workloads[0], &offers(), &[]);
    assert!(
        plan.blocking().is_empty(),
        "storefront should deploy, but: {:?}",
        plan.blocking()
    );
}

#[test]
fn formatting_is_idempotent() {
    for src in [
        include_str!("../examples/storefront.bwk"),
        include_str!("../examples/policies/eu-only.bwk"),
        include_str!("../examples/storefront-eu-policy.bwk"),
        include_str!("../examples/catalog/core.bwk"),
    ] {
        let (once, d1) = bailiwick::format_src(src);
        assert!(!any_errors(&d1), "{:?}", d1);
        let (twice, d2) = bailiwick::format_src(&once);
        assert!(!any_errors(&d2), "{:?}", d2);
        assert_eq!(once, twice, "fmt must be idempotent");
    }
}

#[test]
fn ir_is_stable_and_versioned() {
    let src = include_str!("../examples/storefront.bwk");
    let (file, diags) = parse_and_check(src);
    assert!(!any_errors(&diags));
    let a = bailiwick::ir::to_string(&file);
    let b = bailiwick::ir::to_string(&file);
    assert_eq!(a, b, "IR must be deterministic");
    assert!(a.contains("\"version\": \"bwk/0\""));
    let (slo, _) = parse_and_check("workload w { service s : backend-api { slo.p95 <= 0.2 s from eu } }");
    let slo = bailiwick::ir::to_string(&slo);
    assert!(slo.contains("\"slo_p95_ms\": 200"), "durations are milliseconds: {}", slo);
    assert!(slo.contains("\"slo_from\": [\"eu\"]"), "{}", slo);
    assert!(a.contains("\"deadline_ms\": 21600000"), "6 h = 21,600,000 ms: {}", a);
    assert!(a.contains("\"amount_usd\": 40000"));
}
