//! Resource sizing: data sizes and counts are typed, offers must declare
//! and meet every required minimum, and the plan records which resource
//! fell short. Also covers `image` and `value_of`.

use bailiwick::feasibility::{plan_workload, Exclusion};
use bailiwick::{any_errors, parse_and_check, parse_offers};

fn offers() -> Vec<bailiwick::Offer> {
    let (offers, diags) = parse_offers(include_str!("../examples/catalog/core.bwk"));
    assert!(!any_errors(&diags), "{:?}", diags);
    offers
}

fn errors_contain(src: &str, needle: &str) {
    let (_f, diags) = parse_and_check(src);
    assert!(
        diags.iter().any(|d| d.is_error() && d.msg.contains(needle)),
        "expected an error containing `{}`, got {:?}",
        needle,
        diags
    );
}

#[test]
fn data_size_requires_a_unit() {
    errors_contain(
        "workload w { service s : backend-api { resources { mem >= 4 } } }",
        "data size needs a unit",
    );
}

#[test]
fn data_size_rejects_duration_units() {
    errors_contain(
        "workload w { service s : backend-api { resources { mem >= 200 ms } } }",
        "duration unit",
    );
}

#[test]
fn data_size_rejects_money() {
    errors_contain(
        "workload w { service s : backend-api { resources { disk >= $4 } } }",
        "money",
    );
}

#[test]
fn counts_take_no_unit() {
    errors_contain(
        "workload w { service s : backend-api { resources { vcpu >= 2 GB } } }",
        "takes no unit",
    );
}

#[test]
fn offer_resources_use_equals_and_units_use_ge() {
    errors_contain(
        "offer o { country = de  trust = T2  classes = [ backend-api ]  resources { vcpu >= 2 } }",
        "an offer states what it provides",
    );
    errors_contain(
        "workload w { service s : backend-api { resources { vcpu = 2 } } }",
        "workload resources are minimums",
    );
}

#[test]
fn data_sizes_normalize_to_gb() {
    let src = r#"
        workload w {
          service s : backend-api { resources { mem >= 2048 MB  disk >= 1 TB  vcpu >= 0.5 } }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let r = &f.workloads[0].services[0].resources;
    assert_eq!(r.mem_gb, Some(2.0));
    assert_eq!(r.disk_gb, Some(1024.0));
    assert_eq!(r.vcpu, Some(0.5));
}

#[test]
fn gpu_shortfall_is_reported_with_have_and_need() {
    let src = r#"
        workload w {
          job train : ml-inference { resources { gpu >= 2 } }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &[]);
    let u = &plan.units[0];
    assert!(u.feasible_ids().contains(&"e-us-va"), "{:?}", u.feasible_ids());
    let gpu_offer = u.verdicts.iter().find(|v| v.offer_id == "i-de").unwrap();
    assert!(!gpu_offer.feasible());
    assert!(gpu_offer.exclusions.iter().any(|e| matches!(
        e,
        Exclusion::Resource { resource, need, have: Some(h), .. }
            if resource == "gpu" && *need == 2.0 && *h == 1.0
    )), "{:?}", gpu_offer.exclusions);
    assert!(gpu_offer.exclusions[0].reason().contains("resources.gpu is 1, below the required 2"));
}

#[test]
fn undeclared_resources_exclude() {
    let catalog = r#"
        offer bare { vendor = x  country = de  trust = T3  classes = [ backend-api ] }
        offer sized { vendor = y  country = de  trust = T3  classes = [ backend-api ]  resources { vcpu = 4 } }
    "#;
    let (offers, od) = parse_offers(catalog);
    assert!(!any_errors(&od), "{:?}", od);
    let src = "workload w { service s : backend-api { resources { vcpu >= 1 } } }";
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags));
    let plan = plan_workload(&f.workloads[0], &offers, &[]);
    let u = &plan.units[0];
    assert_eq!(u.feasible_ids(), vec!["sized"]);
    let bare = u.verdicts.iter().find(|v| v.offer_id == "bare").unwrap();
    assert!(matches!(
        &bare.exclusions[0],
        Exclusion::Resource { have: None, .. }
    ));
    assert!(bare.exclusions[0].reason().contains("does not declare resources.vcpu"));
}

#[test]
fn units_without_requirements_ignore_offer_resources() {
    // No `resources` on the unit → rule 6 never fires, even for offers
    // that declare nothing (g-us).
    let src = "workload w { service m : media-assets { } }";
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags));
    let plan = plan_workload(&f.workloads[0], &offers(), &[]);
    assert!(plan.units[0].feasible_ids().contains(&"g-us"));
}

#[test]
fn value_of_is_normalized_to_usd_per_ms_per_request() {
    let src = r#"
        workload w {
          value_of latency = $1.50 per 10 ms per 1000000 requests
          service s : backend-api { value_of latency = $3 per 1 s per 1000 requests }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let w = &f.workloads[0];
    let v = w.value_latency.unwrap();
    assert!((v - 1.5e-7).abs() < 1e-15, "{}", v);
    let sv = w.services[0].value_latency.unwrap();
    assert!((sv - 3.0 / 1000.0 / 1000.0).abs() < 1e-15, "{}", sv);
}

#[test]
fn value_of_rejects_wrong_units() {
    errors_contain(
        "workload w { value_of latency = $1 per 10 GB per 1000 requests }",
        "data-size unit",
    );
    errors_contain(
        "workload w { value_of weight = $1 per 10 ms per 1000 requests }",
        "only `value_of latency`",
    );
}

#[test]
fn image_must_be_a_string() {
    errors_contain(
        "workload w { service s : backend-api { image = nginx } }",
        "quoted string",
    );
    let (f, diags) = parse_and_check(
        r#"workload w { service s : backend-api { image = "registry.example.com/acme/web@sha256:abc" } }"#,
    );
    assert!(!any_errors(&diags), "{:?}", diags);
    assert_eq!(
        f.workloads[0].services[0].image.as_deref(),
        Some("registry.example.com/acme/web@sha256:abc")
    );
}

#[test]
fn fmt_roundtrips_resources_image_and_value_of() {
    let src = include_str!("../examples/storefront.bwk");
    let (once, d1) = bailiwick::format_src(src);
    assert!(!any_errors(&d1), "{:?}", d1);
    let (twice, d2) = bailiwick::format_src(&once);
    assert!(!any_errors(&d2), "{:?}", d2);
    assert_eq!(once, twice);
    assert!(once.contains("resources { vcpu >= 2  mem >= 4 GB }"), "{}", once);
    assert!(once.contains("value_of latency = $1.5 per 10 ms per 1000000 requests"), "{}", once);
    // Values survive the round trip exactly.
    let (a, _) = parse_and_check(src);
    let (b, _) = parse_and_check(&once);
    assert_eq!(a.workloads[0].value_latency, b.workloads[0].value_latency);
    assert_eq!(a.workloads[0].services[0].resources, b.workloads[0].services[0].resources);
    assert_eq!(a.workloads[0].services[0].image, b.workloads[0].services[0].image);
}

#[test]
fn ir_carries_resources_image_and_value_of() {
    let (f, diags) = parse_and_check(include_str!("../examples/storefront.bwk"));
    assert!(!any_errors(&diags));
    let ir = bailiwick::ir::to_string(&f);
    assert!(ir.contains("\"mem_gb\": 4"), "{}", ir);
    assert!(ir.contains("\"vcpu\": 2"), "{}", ir);
    assert!(ir.contains("\"image\": \"registry.example.com/acme/storefront-web:1.0\""), "{}", ir);
    assert!(ir.contains("\"value_latency_usd_per_ms_per_request\": 0.00000015"), "{}", ir);
}
