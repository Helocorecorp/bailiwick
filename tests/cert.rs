//! Deployment certificates: issued only for legal placements, reproducible
//! from their sources, and rejected when anything about them changes.

use std::path::Path;

use bailiwick::cert::{self, Placement};
use bailiwick::json::Json;
use bailiwick::loader;

fn load(case_src: &str) -> (bailiwick::File, Vec<bailiwick::Offer>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/conformance");
    let catalog = loader::load(&dir.join("catalog.bwk"), Vec::new());
    let (file, diags) = bailiwick::parse_and_check(case_src);
    assert!(!bailiwick::any_errors(&diags), "{:?}", diags);
    (file, catalog.file.offers)
}

const CLINIC: &str = include_str!("../spec/conformance/cases/jurisdiction-custody.bwk");

fn place(s: &str) -> Placement {
    Placement::parse(s).unwrap()
}

#[test]
fn a_legal_placement_is_certified_and_verifies() {
    let (file, offers) = load(CLINIC);
    let c = cert::certify(&file, &offers, &[place("api=e-de-he-ck"), place("clinic.reports=a-de")]).unwrap();
    assert_eq!(c.get("format").and_then(Json::as_str), Some(cert::FORMAT));
    assert_eq!(c.get("deployable").and_then(Json::as_bool), Some(true));
    assert_eq!(cert::inputs_digest_len(&c), 64);
    cert::verify(&c, &file, &offers).unwrap();
    // Round-trips through text, as an auditor would receive it.
    let text = bailiwick::json::to_string_pretty(&c);
    cert::verify(&bailiwick::json::parse(&text).unwrap(), &file, &offers).unwrap();
}

#[test]
fn an_illegal_placement_is_never_certified() {
    let (file, offers) = load(CLINIC);
    // Within reach of a denied jurisdiction, and the provider holds the keys.
    let e = cert::certify(&file, &offers, &[place("api=e-de-he")]).unwrap_err();
    assert!(e.contains("refusing to certify") && e.contains("jurisdiction `us`"), "{}", e);
    // Unknown names are errors, not silently ignored.
    assert!(cert::certify(&file, &offers, &[place("nope=a-de")]).is_err());
    assert!(cert::certify(&file, &offers, &[place("api=nope")]).is_err());
}

#[test]
fn any_change_is_detected() {
    let (file, offers) = load(CLINIC);
    let c = cert::certify(&file, &offers, &[place("api=e-de-he-ck")]).unwrap();

    // The sources changed: custody no longer required.
    let (changed, _) = load(&CLINIC.replace("keys        = customer", "keys        = vendor"));
    assert!(cert::verify(&c, &changed, &offers).is_err());

    // A verdict was edited.
    let mut edited = c.clone();
    if let Json::Obj(fs) = &mut edited {
        let units = fs.iter_mut().find(|(k, _)| k == "units").unwrap();
        if let Json::Arr(us) = &mut units.1 {
            if let Json::Obj(u) = &mut us[0] {
                let v = u.iter_mut().find(|(k, _)| k == "deploys").unwrap();
                v.1 = Json::Bool(false);
            }
        }
    }
    assert!(cert::verify(&edited, &file, &offers).is_err());
}

#[test]
fn placements_parse() {
    assert!(Placement::parse("api").is_err());
    assert!(Placement::parse("=a").is_err());
    assert!(Placement::parse("api=").is_err());
    assert_eq!(Placement::parse("api = a, b").unwrap().offers, vec!["a", "b"]);
}
