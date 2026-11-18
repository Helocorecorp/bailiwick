//! Jurisdictions are sets: overlapping regions intersect exactly, names
//! are recovered when an intersection equals a named region, disjoint
//! regions conflict, and countries cover their subdivisions.

use bailiwick::feasibility::plan_workload;
use bailiwick::semantics::{intersect_regions, Residency, CONFLICT};
use bailiwick::{any_errors, parse_and_check, parse_offers};

fn offers() -> Vec<bailiwick::Offer> {
    let (offers, diags) = parse_offers(include_str!("../examples/catalog/core.bwk"));
    assert!(!any_errors(&diags), "{:?}", diags);
    offers
}

fn r(name: &str) -> Residency {
    Residency::from_name(name).expect(name)
}

#[test]
fn eu_is_contained_in_eea() {
    assert_eq!(r("eu").intersect(&r("eea")).display(), "eu");
    assert_eq!(r("eea").intersect(&r("eu")).display(), "eu");
}

#[test]
fn eu_and_nordics_overlap_partially() {
    let x = r("eu").intersect(&r("nordics"));
    assert_eq!(x.display(), "{dk,fi,se}");
    assert!(x.contains("fi", None));
    assert!(!x.contains("no", None), "Norway is Nordic but not EU");
    assert!(!x.contains("de", None));
}

#[test]
fn nordics_is_contained_in_eea() {
    assert_eq!(r("nordics").intersect(&r("eea")).display(), "nordics");
}

#[test]
fn disjoint_regions_conflict() {
    let x = r("eu").intersect(&r("us"));
    assert!(x.is_empty());
    assert_eq!(x.display(), CONFLICT);
    assert_eq!(intersect_regions(Some("eu"), Some("us")).as_deref(), Some(CONFLICT));
}

#[test]
fn global_is_the_identity() {
    assert_eq!(r("global").intersect(&r("eu")).display(), "eu");
    assert_eq!(r("eu").intersect(&r("global")).display(), "eu");
    assert_eq!(r("global").display(), "global");
    assert!(r("global").contains("anything", None));
    assert_eq!(intersect_regions(None, None), None);
    assert_eq!(intersect_regions(Some("global"), None).as_deref(), Some("global"));
}

#[test]
fn unknown_names_are_not_regions() {
    assert!(Residency::from_name("atlantis").is_none());
    let (_f, diags) = parse_and_check("workload w { service s : backend-api { residency = atlantis } }");
    assert!(
        diags.iter().any(|d| d.msg.contains("nordics") && d.msg.contains("atlantis")),
        "the error should list the valid names: {:?}",
        diags
    );
}

#[test]
fn plan_shows_the_explicit_set_and_filters_by_country() {
    let src = r#"
        policy nordics-only { residency = nordics }
        workload w {
          service api : backend-api { residency = eu }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &f.policies);
    let u = &plan.units[0];
    assert_eq!(u.effective_residency, "{dk,fi,se}");
    assert_eq!(u.feasible_ids(), vec!["d-fi"]);
    let vendor_a = u.verdicts.iter().find(|v| v.offer_id == "a-de").unwrap();
    assert!(vendor_a.exclusions[0].reason().contains("outside residency `{dk,fi,se}`"));
}

#[test]
fn a_wider_policy_leaves_a_narrower_unit_unchanged() {
    let src = r#"
        policy eea-only { residency = eea }
        workload w {
          service api : backend-api { residency = eu }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &f.policies);
    assert_eq!(plan.units[0].effective_residency, "eu");
}

#[test]
fn overlapping_policies_are_not_a_conflict() {
    let src = r#"
        policy a { residency = eea }
        policy b { residency = nordics }
        workload w { service api : backend-api { } }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &f.policies);
    assert_eq!(plan.units[0].effective_residency, "nordics");
}

// --- sub-national --------------------------------------------------------------------

#[test]
fn a_country_covers_its_subdivisions() {
    assert_eq!(r("us").intersect(&r("california")).display(), "california");
    assert_eq!(r("california").intersect(&r("us")).display(), "california");
    assert_eq!(r("ca").intersect(&r("quebec")).display(), "quebec");
}

#[test]
fn different_subdivisions_and_countries_conflict() {
    assert!(r("california").intersect(&r("quebec")).is_empty());
    assert!(r("california").intersect(&r("eu")).is_empty());
    assert!(r("california").intersect(&r("ca")).is_empty(), "`ca` is Canada, not California");
}

#[test]
fn a_subnational_residency_needs_the_subdivision() {
    let cal = r("california");
    assert!(cal.is_subnational());
    assert!(!cal.contains("us", None), "unknown subdivision is not admitted");
    assert!(cal.contains("us", Some("ca")));
    assert!(!cal.contains("us", Some("ny")));
    assert!(cal.has_subdivisions_of("us"));
    assert!(!cal.has_subdivisions_of("ca"));
    // A national residency admits any subdivision of the country.
    assert!(r("us").contains("us", Some("ca")));
    assert!(r("us").contains("us", None));
    assert!(!r("us").is_subnational());
}

#[test]
fn plan_explains_subnational_exclusions() {
    let src = r#"
        policy california-only { residency = california }
        workload w { service api : backend-api { } }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &f.policies);
    let u = &plan.units[0];
    assert_eq!(u.effective_residency, "california");
    assert_eq!(u.feasible_ids(), vec!["f-us-ca"]);
    let vendor_e = u.verdicts.iter().find(|v| v.offer_id == "e-us-va").unwrap();
    assert_eq!(vendor_e.exclusions[0].reason(), "location `us-va` is outside residency `california`");
    let cf = u.verdicts.iter().find(|v| v.offer_id == "g-us").unwrap();
    assert!(cf.exclusions.iter().any(|e| e.reason()
        == "location `us` is outside residency `california` (subdivision unknown)"), "{:?}", cf.exclusions);
    let vendor_a = u.verdicts.iter().find(|v| v.offer_id == "a-de").unwrap();
    assert_eq!(vendor_a.exclusions[0].reason(), "location `de` is outside residency `california`");
}

#[test]
fn national_residency_wording_is_unchanged_by_subdivisions() {
    let src = "workload w { service api : backend-api { residency = eu } }";
    let (f, _) = parse_and_check(src);
    let plan = plan_workload(&f.workloads[0], &offers(), &[]);
    let vendor_e = plan.units[0].verdicts.iter().find(|v| v.offer_id == "e-us-va").unwrap();
    assert_eq!(vendor_e.exclusions[0].reason(), "country `us` is outside residency `eu`");
}
