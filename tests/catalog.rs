//! The open catalog: it loads cleanly through imports, ids are unique,
//! every offer is complete, and the worked examples deploy against it.

use std::path::Path;

use bailiwick::{any_errors, loader, plan_file};

fn all() -> bailiwick::File {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loaded = loader::load(&root.join("examples/catalog/all.bwk"), Vec::new());
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    for d in &loaded.diags {
        assert!(!d.msg.contains("unknown feature"), "catalog uses an unknown feature: {}", d.msg);
    }
    loaded.file
}

#[test]
fn the_whole_catalog_loads_without_errors() {
    let file = all();
    assert!(file.offers.len() >= 40, "expected a broad catalog, got {}", file.offers.len());
    let mut ids: Vec<&str> = file.offers.iter().map(|o| o.id.as_str()).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "offer ids must be unique across files");
    for o in &file.offers {
        assert!(o.country.is_some() && o.trust.is_some() && !o.classes.is_empty(), "{} incomplete", o.id);
    }
    // Prices are optional: an offer whose price is not published is left
    // unpriced rather than given an invented one. Most offers have one.
    let priced = file.offers.iter().filter(|o| o.price.is_some()).count();
    assert!(priced * 10 >= file.offers.len() * 9, "{} of {} offers priced", priced, file.offers.len());
}

#[test]
fn every_class_and_region_has_supply() {
    let file = all();
    for class in bailiwick::semantics::CLASSES {
        assert!(file.offers.iter().any(|o| o.classes.iter().any(|c| c == class)), "no offer serves {}", class);
    }
    for region in ["eu", "us", "ca", "uk", "apac", "nordics"] {
        let r = bailiwick::semantics::Residency::from_name(region).unwrap();
        assert!(
            file.offers.iter().any(|o| r.contains(o.country.as_deref().unwrap(), o.subdivision.as_deref())),
            "no offer inside {}",
            region
        );
    }
}

#[test]
fn examples_deploy_against_the_whole_catalog() {
    let file = all();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for example in ["examples/storefront.bwk", "examples/storefront-eu-policy.bwk", "examples/edge-site.bwk"] {
        let loaded = loader::load(&root.join(example), Vec::new());
        assert!(!any_errors(&loaded.diags), "{}: {:?}", example, loaded.diags);
        let plans = plan_file(&loaded.file, &file.offers);
        for p in &plans {
            assert!(p.blocking().is_empty(), "{}: {:?}", example, p.blocking());
        }
    }
}

#[test]
fn a_swiss_offer_is_feasible_only_under_global() {
    let file = all();
    let (w, diags) = bailiwick::parse_and_check(
        "workload w { service a : backend-api { residency = eu }  service b : backend-api { } }",
    );
    assert!(!any_errors(&diags));
    let plans = plan_file(&w, &file.offers);
    assert!(!plans[0].units[0].feasible_ids().contains(&"t-ch"));
    assert!(plans[0].units[1].feasible_ids().contains(&"t-ch"));
}
