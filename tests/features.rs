//! Edge cache rules and features: cache rules parse and normalize, imply
//! the `cache` feature, and a required feature excludes offers that do not
//! list it (rule 7).

use bailiwick::feasibility::{plan_workload, Exclusion};
use bailiwick::{any_errors, parse_and_check, parse_offers};

fn offers() -> Vec<bailiwick::Offer> {
    let (offers, diags) = parse_offers(include_str!("../examples/catalog/core.bwk"));
    assert!(!any_errors(&diags), "{:?}", diags);
    offers
}

#[test]
fn cache_rules_parse_and_normalize() {
    let src = r#"
        workload w {
          service www : frontend-static {
            cache "/assets/*" for 30 d
            cache "/" for 5 m
            cache "/api/*" never
          }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let s = &f.workloads[0].services[0];
    assert_eq!(s.cache.len(), 3);
    assert_eq!(s.cache[0].path, "/assets/*");
    assert_eq!(s.cache[0].ttl_ms, Some(30.0 * 86_400_000.0));
    assert_eq!(s.cache[1].ttl_ms, Some(300_000.0));
    assert_eq!(s.cache[2].ttl_ms, None, "`never` is no TTL");
}

#[test]
fn cache_path_must_be_absolute() {
    let (_f, diags) = parse_and_check(
        "workload w { service s : frontend-static { cache \"assets/*\" for 1 h } }",
    );
    assert!(diags.iter().any(|d| d.is_error() && d.msg.contains("must start with `/`")), "{:?}", diags);
}

#[test]
fn cache_needs_for_or_never() {
    let (_f, diags) = parse_and_check("workload w { service s : frontend-static { cache \"/x\" 30 d } }");
    assert!(diags.iter().any(|d| d.is_error() && d.msg.contains("`for <duration>` or `never`")), "{:?}", diags);
    let (_f, diags) = parse_and_check("workload w { service s : frontend-static { cache \"/x\" for $5 } }");
    assert!(diags.iter().any(|d| d.is_error() && d.msg.contains("duration")), "{:?}", diags);
}

#[test]
fn duplicate_cache_rules_warn() {
    let (_f, diags) = parse_and_check(
        "workload w { service s : frontend-static { cache \"/x\" for 1 h  cache \"/x\" never } }",
    );
    assert!(!any_errors(&diags));
    assert!(diags.iter().any(|d| d.msg.contains("more than one cache rule")), "{:?}", diags);
}

#[test]
fn cache_rules_imply_the_cache_feature() {
    let (f, _) = parse_and_check("workload w { service s : frontend-static { cache \"/x\" for 1 h } }");
    assert_eq!(f.workloads[0].services[0].required_features(), vec!["cache".to_string()]);
    let (f, _) = parse_and_check(
        "workload w { service s : frontend-static { requires = [ cache, waf ]  cache \"/x\" for 1 h } }",
    );
    assert_eq!(
        f.workloads[0].services[0].required_features(),
        vec!["cache".to_string(), "waf".to_string()],
        "no duplicate when `cache` is already explicit"
    );
}

#[test]
fn feature_rule_excludes_offers_without_the_feature() {
    let src = r#"
        workload site {
          service www : frontend-static { cache "/assets/*" for 30 d }
          service media : media-assets { requires = [ cache ] }
        }
    "#;
    let (f, diags) = parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &[]);
    let www = &plan.units[0];
    assert_eq!(www.requires, vec!["cache".to_string()]);
    assert_eq!(www.feasible_ids(), vec!["g-us", "h-si"]);
    let media = &plan.units[1];
    assert_eq!(media.feasible_ids(), vec!["g-us", "h-si"]);
    let vendor_a = media.verdicts.iter().find(|v| v.offer_id == "a-de").unwrap();
    assert!(matches!(&vendor_a.exclusions[0], Exclusion::Feature { feature } if feature == "cache"));
    assert_eq!(vendor_a.exclusions[0].reason(), "does not provide feature `cache`");
    assert_eq!(vendor_a.exclusions[0].kind(), "feature");
    assert!(plan.blocking().is_empty());
}

#[test]
fn a_required_feature_nobody_provides_fails_closed() {
    let (f, diags) = parse_and_check(
        "workload w { service s : media-assets { requires = [ image-resizing ] } }",
    );
    assert!(!any_errors(&diags), "{:?}", diags);
    let plan = plan_workload(&f.workloads[0], &offers(), &[]);
    assert!(!plan.blocking().is_empty());
    assert!(plan.blocking()[0].contains("no feasible offer"));
}

#[test]
fn unknown_features_warn_but_do_not_error() {
    let (_f, diags) = parse_and_check("workload w { service s : backend-api { requires = [ teleport ] } }");
    assert!(!any_errors(&diags), "{:?}", diags);
    assert!(diags.iter().any(|d| d.msg.contains("unknown feature `teleport`")), "{:?}", diags);
    let (_f, diags) = parse_and_check(
        "offer o { country = de  trust = T2  classes = [ backend-api ]  features = [ warp ] }",
    );
    assert!(!any_errors(&diags), "{:?}", diags);
    assert!(diags.iter().any(|d| d.msg.contains("unknown feature `warp`")), "{:?}", diags);
}

#[test]
fn fmt_and_ir_carry_cache_rules_and_features() {
    let src = include_str!("../examples/edge-site.bwk");
    let (once, d1) = bailiwick::format_src(src);
    assert!(!any_errors(&d1), "{:?}", d1);
    let (twice, d2) = bailiwick::format_src(&once);
    assert!(!any_errors(&d2));
    assert_eq!(once, twice, "fmt must be idempotent");
    assert!(once.contains("cache \"/assets/*\" for 30 d"), "{}", once);
    assert!(once.contains("cache \"/api/*\" never"), "{}", once);
    assert!(once.contains("requires    = [ cache, image-resizing ]"), "{}", once);

    let (f, _) = parse_and_check(src);
    let ir = bailiwick::ir::to_string(&f);
    assert!(ir.contains("\"path\": \"/assets/*\""), "{}", ir);
    assert!(ir.contains("\"ttl_ms\": 2592000000"), "{}", ir);
    assert!(ir.contains("\"ttl_ms\": null"), "{}", ir);
    assert!(ir.contains("\"requires\": [\"http3\"]"), "{}", ir);

    let (f, _) = parse_and_check(include_str!("../examples/catalog/core.bwk"));
    let ir = bailiwick::ir::to_string(&f);
    assert!(ir.contains("\"features\": [\"cache\", \"waf\", \"http3\"]"), "{}", ir);
    assert!(ir.contains("\"subdivision\": \"va\""), "{}", ir);
}
