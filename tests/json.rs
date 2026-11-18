//! The in-tree JSON parser: it reads what the printer writes, handles
//! escapes and surrogate pairs, and rejects malformed input.

use bailiwick::json::{self, Json};
use bailiwick::{any_errors, parse_and_check};

#[test]
fn parses_what_it_prints() {
    let (file, diags) = parse_and_check(include_str!("../examples/storefront.bwk"));
    assert!(!any_errors(&diags));
    let printed = bailiwick::ir::to_string(&file);
    let parsed = json::parse(&printed).expect("parse");
    assert_eq!(json::to_string_pretty(&parsed), printed);
    assert_eq!(parsed.get("version").and_then(|v| v.as_str()), Some("bwk/0"));
    let compact = json::to_string_compact(&parsed);
    assert!(!compact.contains('\n'));
    assert_eq!(json::parse(&compact).unwrap(), parsed);
}

#[test]
fn parses_escapes_and_unicode() {
    let v = json::parse(r#"{"s": "a\"b\\c\n\u00e9\ud83d\ude00", "n": -1.5e2, "t": true, "z": null, "a": [1, [2]]}"#).unwrap();
    assert_eq!(v.get("s").and_then(|s| s.as_str()), Some("a\"b\\c\né😀"));
    assert_eq!(v.get("n").and_then(|n| n.as_f64()), Some(-150.0));
    assert_eq!(v.get("t").and_then(|t| t.as_bool()), Some(true));
    assert_eq!(v.get("z"), Some(&Json::Null));
    assert_eq!(v.get("a").and_then(|a| a.as_arr()).map(|a| a.len()), Some(2));
}

#[test]
fn rejects_malformed_input() {
    assert!(json::parse("{\"a\": 1,}").is_err(), "trailing comma");
    assert!(json::parse("[1 2]").is_err());
    assert!(json::parse("\"open").is_err());
    assert!(json::parse("{\"a\": 1} x").is_err(), "trailing characters");
    assert!(json::parse("tru").is_err());
    assert!(json::parse("\"\\ud83d\"").is_err(), "unpaired surrogate");
    let deep = "[".repeat(300) + &"]".repeat(300);
    assert!(json::parse(&deep).unwrap_err().contains("nesting too deep"));
}
