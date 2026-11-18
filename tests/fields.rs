//! Single-valued fields may be written only once, number literals are
//! bounded, and one syntax error yields one diagnostic, not a cascade.

use bailiwick::{any_errors, parse_and_check, parse_offers};

fn errors(src: &str) -> Vec<String> {
    let (_f, diags) = parse_and_check(src);
    diags.into_iter().filter(|d| d.is_error()).map(|d| d.msg).collect()
}

#[test]
fn a_second_residency_is_an_error_not_an_override() {
    // Last-one-wins would silently move this data from the EU to the US.
    let errs = errors(
        r#"
        workload w {
          data d { sensitivity = S3  residency = eu  residency = us }
          service s : backend-api { depends_on = d }
        }
    "#,
    );
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("`residency` is set more than once in data `d`"), "{:?}", errs);
}

#[test]
fn every_single_valued_block_rejects_repeats() {
    let cases = [
        ("policy p { min_trust = T2 min_trust = T1 }", "`min_trust`"),
        ("workload w { profile = a profile = b }", "`profile`"),
        ("workload w { service s : backend-api { keys = customer keys = vendor } }", "`keys`"),
        ("workload w { service s : backend-api { slo.p95 <= 200 ms slo.p95 <= 2 s } }", "`slo`"),
        ("workload w { job j : batch { sensitivity = S3 sensitivity = S1 } }", "`sensitivity`"),
        ("workload w { service s : backend-api { resources { mem >= 4 GB mem >= 1 TB } } }", "`mem`"),
    ];
    for (src, field) in cases {
        let errs = errors(src);
        assert!(
            errs.iter().any(|e| e.contains(field) && e.contains("more than once")),
            "{}: {:?}",
            src,
            errs
        );
    }
}

#[test]
fn an_offer_field_written_twice_is_an_error() {
    let (_o, diags) = parse_offers("offer o { vendor = a country = de country = us trust = T3 }");
    assert!(
        diags.iter().any(|d| d.is_error() && d.msg.contains("`country` is set more than once")),
        "{:?}",
        diags
    );
}

#[test]
fn accumulating_fields_may_repeat() {
    let (_f, diags) = parse_and_check(
        r#"
        workload w {
          data a { residency = eu }
          data b { sensitivity = S3 }
          service s : backend-api {
            depends_on = a
            depends_on = b
            cache "/a/*" for 1 h
            cache "/b/*" never
          }
        }
        policy p { deny vendors = [x]  deny vendors = [y] }
    "#,
    );
    assert!(!any_errors(&diags), "{:?}", diags);
}

#[test]
fn numbers_above_2_pow_53_are_rejected() {
    let errs = errors("workload w { service s : backend-api { resources { vcpu >= 99999999999999999999 } } }");
    assert!(errs.iter().any(|e| e.contains("too large")), "{:?}", errs);
    let ok = errors("workload w { service s : backend-api { resources { vcpu >= 9007199254740992 } } }");
    assert!(ok.is_empty(), "{:?}", ok);
}

#[test]
fn one_syntax_error_gives_one_diagnostic() {
    // `slo { ... }` instead of `slo.p95 <= ...` used to produce seven
    // errors, most of them about the lines after it.
    let errs = errors(
        r#"
        workload shop {
          data customers { sensitivity = S3  residency = eu }
          service api : backend-api {
            slo { p95 <= 5 GB }
            depends_on = customers
          }
        }
    "#,
    );
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("expected `.` after `slo`"), "{:?}", errs);
}
