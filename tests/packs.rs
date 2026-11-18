//! The built-in compliance packs: each one loads cleanly, and each does
//! what its header says against real catalog offers.

use bailiwick::builtin;
use bailiwick::{any_errors, plan_file};

fn feasible(src: &str) -> Vec<String> {
    let (file, diags) = bailiwick::parse_and_check(src);
    assert!(!any_errors(&diags), "{:?}", diags);
    let loaded = bailiwick::loader::load_source(std::path::Path::new("t.bwk"), src, Vec::new());
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    let _ = file;
    let plans = plan_file(&loaded.file, &builtin::example_offers());
    plans[0].units[0].feasible_ids().into_iter().map(String::from).collect()
}

fn with(pack: &str) -> Vec<String> {
    feasible(&format!("import \"{}\"\nworkload w {{ service s : backend-api {{ }} }}\n", pack))
}

#[test]
fn every_pack_loads_without_problems() {
    for (name, src) in builtin::PACKS {
        let (file, diags) = bailiwick::parse_and_check(src);
        assert!(diags.is_empty(), "pack `{}`: {:?}", name, diags);
        assert_eq!(file.policies.len(), 1, "pack `{}` defines one policy", name);
        assert_eq!(file.policies[0].name, *name, "pack `{}` names its policy after itself", name);
        assert!(file.workloads.is_empty() && file.offers.is_empty(), "pack `{}` holds only a policy", name);
        assert!(src.contains("Not legal advice"), "pack `{}` says it is not legal advice", name);
    }
}

#[test]
fn gdpr_keeps_data_in_the_eea_or_adequate_countries() {
    let f = with("gdpr");
    assert!(f.contains(&"a-de".to_string()));
    assert!(f.contains(&"e-gb-eng".to_string()), "the UK has adequacy");
    assert!(f.contains(&"e-ca-qc".to_string()), "Canada has adequacy");
    assert!(!f.iter().any(|o| o.starts_with("e-us") || o == "f-us-ia"), "{:?}", f);
    assert!(!f.contains(&"e-sg".to_string()), "Singapore has no adequacy");
}

#[test]
fn eu_sovereign_admits_us_vendors_only_with_customer_keys() {
    let f = with("eu-sovereign");
    assert!(f.contains(&"e-de-he-ck".to_string()));
    assert!(f.contains(&"f-de-he-ck".to_string()));
    assert!(!f.contains(&"e-de-he".to_string()));
    assert!(!f.contains(&"j-nl-nh".to_string()));
    assert!(f.contains(&"b-fr".to_string()));
}

#[test]
fn quebec_law25_needs_a_quebec_subdivision() {
    let f = with("quebec-law25");
    // Every example offer in Québec, and nothing elsewhere in Canada.
    assert_eq!(f, vec!["e-ca-qc", "f-ca-qc", "c-ca-qc"]);
    assert!(!f.iter().any(|o| o == "k-ca-on" || o == "j-ca-on"));
}

#[test]
fn location_neutral_laws_do_not_restrict_location() {
    for p in ["pipeda", "hipaa", "ccpa"] {
        let f = with(p);
        assert!(f.iter().any(|o| o.starts_with("e-us")), "{} allows the US", p);
        assert!(f.contains(&"a-de".to_string()), "{} allows Germany", p);
    }
    let both = feasible("import \"pipeda\"\nimport \"canada-residency\"\nworkload w { service s : backend-api { } }\n");
    assert!(!both.is_empty() && both.iter().all(|o| o.contains("-ca-") || o.ends_with("-ca")), "{:?}", both);
}

#[test]
fn an_unknown_pack_is_an_error_naming_the_real_ones() {
    let loaded = bailiwick::loader::load_source(std::path::Path::new("t.bwk"), "import \"gdrp\"\n", Vec::new());
    assert!(any_errors(&loaded.diags));
    assert!(loaded.diags.iter().any(|d| d.msg.contains("gdpr, eu-sovereign")));
}
