//! The Protobuf codec: files round-trip losslessly, encoding is
//! deterministic and versioned, plans encode, and garbage is rejected
//! without panicking.

use std::path::Path;

use bailiwick::{any_errors, ir, loader, parse_and_check, plan_file, proto};

#[test]
fn file_round_trips_through_protobuf() {
    let (file, diags) = parse_and_check(include_str!("../examples/storefront.bwk"));
    assert!(!any_errors(&diags));
    let bytes = proto::encode_file(&file);
    let decoded = proto::decode_file(&bytes).expect("decode");
    assert_eq!(ir::to_string(&decoded), ir::to_string(&file), "IR must survive the round trip");
    assert_eq!(proto::encode_file(&decoded), bytes, "re-encoding must be byte-identical");
}

#[test]
fn policies_imports_and_offers_round_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loaded = loader::load(&root.join("examples/storefront-eu-policy.bwk"), Vec::new());
    assert!(!any_errors(&loaded.diags), "{:?}", loaded.diags);
    let (catalog, d) = parse_and_check(include_str!("../examples/catalog/core.bwk"));
    assert!(!any_errors(&d));
    let mut file = loaded.file;
    file.offers = catalog.offers;
    let bytes = proto::encode_file(&file);
    let decoded = proto::decode_file(&bytes).expect("decode");
    assert_eq!(ir::to_string(&decoded), ir::to_string(&file));
    assert_eq!(decoded.policies.len(), 1);
    assert_eq!(decoded.imports[0].path, "policies/eu-only.bwk");
    let cf = decoded.offers.iter().find(|o| o.id == "g-us").unwrap();
    assert_eq!(cf.features, vec!["cache", "waf", "http3"]);
    let vendor_e = decoded.offers.iter().find(|o| o.id == "e-us-va").unwrap();
    assert_eq!(vendor_e.subdivision.as_deref(), Some("va"));
}

#[test]
fn cache_rules_and_features_round_trip() {
    let (file, diags) = parse_and_check(include_str!("../examples/edge-site.bwk"));
    assert!(!any_errors(&diags));
    let decoded = proto::decode_file(&proto::encode_file(&file)).unwrap();
    let s = &decoded.workloads[0].services[0];
    assert_eq!(s.cache.len(), 3);
    assert_eq!(s.cache[0].ttl_ms, Some(30.0 * 86_400_000.0));
    assert_eq!(s.cache[2].ttl_ms, None, "`never` survives via the `never` flag");
    assert_eq!(s.requires, vec!["http3"]);
}

#[test]
fn encoding_is_deterministic_and_versioned() {
    let (file, _) = parse_and_check(include_str!("../examples/storefront.bwk"));
    let a = proto::encode_file(&file);
    let b = proto::encode_file(&file);
    assert_eq!(a, b);
    // Field 1 (string) tag = 0x0a, length 5, "bwk/0".
    assert_eq!(&a[..7], &[0x0a, 0x05, b'b', b'w', b'k', b'/', b'0']);
}

#[test]
fn plans_encode_with_their_version() {
    let (file, _) = parse_and_check(include_str!("../examples/storefront.bwk"));
    let (catalog, _) = parse_and_check(include_str!("../examples/catalog/core.bwk"));
    let plans = plan_file(&file, &catalog.offers);
    let bytes = proto::encode_plans(&plans);
    assert!(bytes.len() > 100);
    assert_eq!(&bytes[..2], &[0x0a, 0x0a]);
    assert_eq!(&bytes[2..12], b"bwk-plan/0");
    assert_eq!(proto::encode_plans(&plans), bytes, "deterministic");
}

#[test]
fn optional_presence_is_preserved() {
    // vcpu = 0 on an offer is a stated zero, not an absence.
    let (file, diags) = parse_and_check(
        "offer o { country = de  trust = T2  classes = [ backend-api ]  resources { vcpu = 0  gpu = 0 } }",
    );
    assert!(!any_errors(&diags));
    let decoded = proto::decode_file(&proto::encode_file(&file)).unwrap();
    assert_eq!(decoded.offers[0].resources.vcpu, Some(0.0));
    assert_eq!(decoded.offers[0].resources.gpu, Some(0.0));
    assert_eq!(decoded.offers[0].resources.mem_gb, None);
}

#[test]
fn decoder_rejects_garbage_without_panicking() {
    assert!(proto::decode_file(&[]).unwrap_err().contains("missing version"));
    assert!(proto::decode_file(&[0xff; 16]).is_err());
    assert!(proto::decode_file(&[0x0a, 0x7f, 0x01]).is_err(), "declared length past the end");
    assert!(proto::decode_file(&[0x0a, 0x05, b'b', b'w', b'k', b'/', b'9']).unwrap_err().contains("unsupported IR version"));
    let (file, _) = parse_and_check(include_str!("../examples/storefront.bwk"));
    let bytes = proto::encode_file(&file);
    for cut in [1usize, 7, 20, bytes.len() / 2, bytes.len() - 1] {
        let _ = proto::decode_file(&bytes[..cut]); // must return, never panic
    }
}
