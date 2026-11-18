//! The adversarial half of the importer suite: what the hand-written YAML,
//! TOML and HCL readers do with input that is malformed, hostile or merely
//! unusual, and what the four importers actually *say* in the BWK they
//! write.
//!
//! `tests/import.rs` covers the shapes these readers claim to support on
//! well-formed input. This file covers the edges, because a defect here
//! does not show up as a syntax error: it mis-states a customer's
//! requirements, and the constraint price we quote is the price of exactly
//! those requirements. The load-bearing assertion is
//! `every_inferred_constraint_carries_a_todo_and_its_evidence` — if a
//! refactor drops a `TODO: confirm` marker, tools downstream start treating
//! guesses as constraints nobody confirmed.
//!
//! Two conventions here. A test whose name says the reader *keeps*,
//! *accepts* or *picks* something is pinning behaviour that is a finding
//! rather than a guarantee; its comment says what a customer would see. A
//! test marked `#[ignore]` cannot pass without changing what this crate
//! means by a parse, and says what that would take.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bailiwick::import::{compose, fly, hcl_min, k8s, terraform, toml_min, yaml_min, Imported};
use bailiwick::{any_errors, parse_and_check};

const FLY: &str = include_str!("../examples/import/fly.toml");
const COMPOSE: &str = include_str!("../examples/import/docker-compose.yml");
const K8S: &str = include_str!("../examples/import/k8s.yaml");
const TF: &str = include_str!("../examples/import/main.tf");

// --- fixtures ---------------------------------------------------------------
//
// Written as raw strings starting in column 0. Indentation *is* the syntax in
// YAML, and a `\` continuation inside an ordinary Rust string swallows the
// next line's leading whitespace, which silently rewrites the fixture into
// something other than the manifest the test means to describe.

/// A pod sized with nested flow collections, as people write by hand.
const FLOW_SIZED_POD: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      containers:
        - name: api
          image: a:1
          resources: {requests: {cpu: 2, memory: 8Gi}}
"#;

/// The same pod, sized in block style.
const BLOCK_SIZED_POD: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      containers:
        - name: api
          image: a:1
          resources:
            requests:
              cpu: 2
              memory: 8Gi
"#;

const FLOW_NODE_SELECTOR: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      nodeSelector: {topology.kubernetes.io/region: us-east-1, disk: ssd}
      containers:
        - name: api
          image: a:1
"#;

/// A Deployment whose container args hold a script containing `---`. The
/// `resources` and `nodeSelector` after it are what gets lost.
const SEPARATOR_IN_BLOCK_SCALAR: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      containers:
        - name: api
          image: a:1
          args:
            - |
              echo one
              ---
              echo two
      nodeSelector:
        topology.kubernetes.io/region: eu-central-1
"#;

const COMPOSE_EXTENDS: &str = r#"
name: x
services:
  base:
    image: registry.example.com/acme/base:1
    deploy:
      resources:
        reservations:
          cpus: '2'
          memory: 2G
  api:
    extends:
      service: base
"#;

/// A pod with a sidecar: 0.5 vCPU plus 1 vCPU of request.
const SIDECAR_POD: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      containers:
        - name: api
          image: a:1
          resources:
            requests:
              cpu: 500m
              memory: 512Mi
        - name: sidecar
          image: envoy:1
          resources:
            requests:
              cpu: '1'
              memory: 1Gi
"#;

/// A region pinned the way Kubernetes documents now, with `nodeAffinity`.
const NODE_AFFINITY_POD: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: shop
spec:
  template:
    spec:
      affinity:
        nodeAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
            nodeSelectorTerms:
              - matchExpressions:
                  - key: topology.kubernetes.io/region
                    operator: In
                    values: [eu-central-1]
      containers:
        - name: api
          image: a:1
"#;

const UNMAPPED_REGION_DEPLOYMENT: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: n
  namespace: shop
spec:
  template:
    spec:
      nodeSelector:
        topology.kubernetes.io/region: xx-nowhere-9
      containers:
        - name: c
          image: a:1
"#;

const UNMAPPED_REGION_CRONJOB: &str = r#"
apiVersion: batch/v1
kind: CronJob
metadata:
  name: n
  namespace: shop
spec:
  schedule: '0 2 * * *'
  jobTemplate:
    spec:
      template:
        spec:
          nodeSelector:
            topology.kubernetes.io/region: xx-nowhere-9
          containers:
            - name: c
              image: a:1
"#;

/// A PersistentVolumeClaim called `db-store` beside a StatefulSet called
/// `db`: both reduce to the BWK name `db-store`.
const CLASHING_NAMES: &str = r#"
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: db-store
  namespace: shop
spec:
  resources:
    requests:
      storage: 10Gi
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: db
  namespace: shop
spec:
  template:
    spec:
      containers:
        - name: db
          image: postgres:16
"#;

/// `NUM` stands in for a digit run too long for `f64`. `1e400` would not
/// do: its `e400Gi` tail is not a unit the reader knows, so it is refused
/// before any overflow can happen, and a test using it would pass whether
/// or not the overflow guard is there at all.
const COMPOSE_OVERFLOW: &str = r#"
name: x
services:
  api:
    image: a:1
    deploy:
      resources:
        reservations:
          cpus: 'NUM'
          memory: 'NUMGi'
"#;

const COMPOSE_SIZED: &str = r#"
name: x
services:
  api:
    image: a:1
    deploy:
      resources:
        reservations:
          cpus: '2'
          memory: 2G
"#;

/// `NUM` as in `COMPOSE_OVERFLOW`.
const K8S_OVERFLOW: &str = r#"
apiVersion: batch/v1
kind: Job
metadata:
  name: j
  namespace: shop
spec:
  activeDeadlineSeconds: 'NUM'
  template:
    spec:
      containers:
        - name: c
          image: a:1
          resources:
            limits:
              nvidia.com/gpu: 'NUM'
"#;

/// A digit run past the range of `f64`, which parses as infinity.
fn overflowing() -> String {
    "9".repeat(320)
}

// --- harness ----------------------------------------------------------------

/// Run `f` on a worker thread with a small stack and a hard time limit.
///
/// A parser that fails to make progress must fail a test rather than hang
/// the suite, so nothing here relies on a parse finishing. The 1 MiB stack
/// is deliberately far smaller than the 8 MiB a test thread normally gets:
/// it means the depth guards are doing real work rather than being masked
/// by a roomy stack. A guard that is missing altogether overflows and
/// aborts the process, which is loud, attributable and the point.
fn bounded<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let h = thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || tx.send(f()).ok())
        .expect("spawn a worker thread");
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(v) => {
            h.join().ok();
            v
        }
        Err(_) => panic!("{}: no result in 20 s — the parser is not making progress", what),
    }
}

/// The fields an importer *infers* rather than reads: residency,
/// sensitivity and a job's checkpointability. Every one of these is a claim
/// about intent that the input does not actually make, so every one must be
/// marked for confirmation. Commented-out suggestions are not constraints.
const INFERRED: [&str; 3] = ["residency", "sensitivity", "checkpointable"];

/// `(field, whole line)` for each `field = value` line in `bwk`.
fn assignments(bwk: &str) -> Vec<(&str, &str)> {
    bwk.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && l.contains('=') && !l.contains(">="))
        .filter_map(|l| l.split('=').next().map(|h| (h.trim(), l)))
        .filter(|(h, _)| !h.is_empty() && h.split_whitespace().count() == 1)
        .collect()
}

fn inferred(bwk: &str) -> Vec<(&str, &str)> {
    assignments(bwk).into_iter().filter(|(h, _)| INFERRED.contains(h)).collect()
}

/// The evidence a `residency` line cites, as the last token of its
/// `; from ...` clause with any quotes removed.
fn cited_evidence(line: &str) -> &str {
    let (_, from) = line
        .split_once("// TODO: confirm; from ")
        .unwrap_or_else(|| panic!("residency line cites no evidence: `{}`", line));
    from.split_whitespace()
        .next_back()
        .unwrap_or("")
        .trim_matches('"')
}

fn ok(tag: &str, r: Result<Imported, String>) -> Imported {
    r.unwrap_or_else(|e| panic!("{} should import: {}", tag, e))
}

/// Assert that the generated BWK parses *and* checks: a skeleton the
/// customer cannot run `bailiwick check` on is of no use to them.
fn assert_checks(tag: &str, bwk: &str) {
    let (_file, diags) = parse_and_check(bwk);
    assert!(!any_errors(&diags), "{}: generated BWK must check: {:?}\n{}", tag, diags, bwk);
}

// --- the load-bearing assertion ---------------------------------------------

/// Every inferred constraint carries `TODO: confirm`, every residency names
/// the input it was read from, and that evidence really is in the input.
///
/// This is the assertion the product rests on. The importer may only guess
/// at intent if it says, at each site, that it guessed and what it guessed
/// from; the counts stop a dropped constraint passing as a clean import.
#[test]
fn every_inferred_constraint_carries_a_todo_and_its_evidence() {
    let cases: [(&str, Imported, &str, usize, usize); 4] = [
        ("fly", ok("fly", fly::import(FLY)), FLY, 3, 2),
        ("compose", ok("compose", compose::import(COMPOSE)), COMPOSE, 2, 0),
        ("k8s", ok("k8s", k8s::import(K8S)), K8S, 4, 1),
        ("terraform", ok("terraform", terraform::import(TF)), TF, 8, 6),
    ];
    for (tag, out, src, want_inferred, want_residency) in &cases {
        let lines = inferred(&out.bwk);
        assert_eq!(
            lines.len(),
            *want_inferred,
            "{}: inferred constraint count changed\n{}",
            tag,
            out.bwk
        );
        for (field, line) in &lines {
            assert!(
                line.contains("// TODO: confirm"),
                "{}: `{}` is inferred but is not marked for confirmation: `{}`",
                tag,
                field,
                line
            );
        }
        let residency: Vec<&(&str, &str)> =
            lines.iter().filter(|(f, _)| *f == "residency").collect();
        assert_eq!(
            residency.len(),
            *want_residency,
            "{}: residency count changed\n{}",
            tag,
            out.bwk
        );
        for (_, line) in &residency {
            let ev = cited_evidence(line);
            assert!(!ev.is_empty(), "{}: empty evidence on `{}`", tag, line);
            assert!(
                src.contains(ev),
                "{}: cited evidence `{}` is not in the input it claims to come from",
                tag,
                ev
            );
        }
    }
}

/// A fact read out of the input is not a guess, and must not be dressed as
/// one. `image`, `deadline` and `depends_on` come straight from the input;
/// only the fields in `INFERRED` are confirmations the customer owes us.
#[test]
fn facts_read_from_the_input_are_not_marked_for_confirmation() {
    for (tag, out) in [
        ("fly", ok("fly", fly::import(FLY))),
        ("compose", ok("compose", compose::import(COMPOSE))),
        ("k8s", ok("k8s", k8s::import(K8S))),
        ("terraform", ok("terraform", terraform::import(TF))),
    ] {
        for (field, line) in assignments(&out.bwk) {
            if INFERRED.contains(&field) {
                continue;
            }
            assert!(
                !line.contains("TODO: confirm"),
                "{}: `{}` is read from the input, so it must not ask to be confirmed: `{}`",
                tag,
                field,
                line
            );
        }
    }
}

#[test]
fn every_importer_writes_bwk_that_checks() {
    assert_checks("fly", &ok("fly", fly::import(FLY)).bwk);
    assert_checks("compose", &ok("compose", compose::import(COMPOSE)).bwk);
    assert_checks("k8s", &ok("k8s", k8s::import(K8S)).bwk);
    assert_checks("terraform", &ok("terraform", terraform::import(TF)).bwk);
}

#[test]
fn importers_are_byte_for_byte_deterministic() {
    for (tag, a, b) in [
        ("fly", fly::import(FLY), fly::import(FLY)),
        ("compose", compose::import(COMPOSE), compose::import(COMPOSE)),
        ("k8s", k8s::import(K8S), k8s::import(K8S)),
        ("terraform", terraform::import(TF), terraform::import(TF)),
    ] {
        let (a, b) = (ok(tag, a), ok(tag, b));
        assert_eq!(a.bwk, b.bwk, "{}: same input must give the same bytes", tag);
        assert_eq!(a.notes, b.notes, "{}: notes must be stable too", tag);
    }
}

// --- yaml: the subset it claims ---------------------------------------------

/// YAML forbids a tab as indentation outright, and this reader knows it:
/// a tab that indents is refused, a tab inside a value is just whitespace.
#[test]
fn yaml_refuses_a_tab_as_indentation_but_allows_one_in_a_value() {
    for src in ["a:\n\tb: 1\n", "a:\n \tb: 1\n"] {
        let err = yaml_min::parse(src).expect_err("a tab that indents must be refused");
        assert!(err.contains("tabs are not allowed for indentation"), "{}", err);
        assert!(err.starts_with("line 2:"), "the diagnostic must name the line: {}", err);
    }
    let n = yaml_min::parse("a: \tx\n").expect("a tab inside a value is whitespace");
    assert_eq!(n.get("a").and_then(|v| v.as_str()), Some("x"));
}

#[test]
fn yaml_reads_crlf_exactly_as_lf() {
    let lf = yaml_min::parse("services:\n  web:\n    image: nginx\n").unwrap();
    let crlf = yaml_min::parse("services:\r\n  web:\r\n    image: nginx\r\n").unwrap();
    assert_eq!(lf, crlf, "a Windows checkout must parse the same as a Unix one");
}

#[test]
fn yaml_empty_and_comment_only_documents_are_null() {
    for src in ["", "\n\n\n", "# just a comment\n", "---\n", "   \n#x\n"] {
        assert_eq!(yaml_min::parse(src), Ok(yaml_min::Node::Null), "{:?}", src);
    }
    assert_eq!(yaml_min::parse_all("---\n# nothing\n---\n# nor here\n"), Ok(vec![]));
}

/// A `#` is only a comment where YAML says it is: after whitespace, outside
/// quotes. An image tag or a password with a `#` in it must survive.
#[test]
fn yaml_keeps_a_hash_that_is_not_a_comment() {
    let n = yaml_min::parse("a: \"x # kept\"\nb: 'y # kept'\nc: plain # dropped\nd: no#space\n")
        .unwrap();
    assert_eq!(n.get("a").and_then(|v| v.as_str()), Some("x # kept"));
    assert_eq!(n.get("b").and_then(|v| v.as_str()), Some("y # kept"));
    assert_eq!(n.get("c").and_then(|v| v.as_str()), Some("plain"));
    assert_eq!(n.get("d").and_then(|v| v.as_str()), Some("no#space"));
}

#[test]
fn yaml_scalars_stay_text_and_null_forms_are_null() {
    let n = yaml_min::parse("i: 7\nf: 1.5\nt: true\nz: ~\nq: 'quoted'\ne: \"\"\nn: null\n").unwrap();
    // This reader has no scalar types: every leaf is text, and the
    // importers do their own number parsing. Nothing here is an `i64`.
    assert_eq!(n.get("i").and_then(|v| v.as_str()), Some("7"));
    assert_eq!(n.get("f").and_then(|v| v.as_str()), Some("1.5"));
    assert_eq!(n.get("t").and_then(|v| v.as_str()), Some("true"));
    assert_eq!(n.get("q").and_then(|v| v.as_str()), Some("quoted"));
    assert_eq!(n.get("e").and_then(|v| v.as_str()), Some(""));
    assert_eq!(n.get("z"), Some(&yaml_min::Node::Null));
    assert_eq!(n.get("n"), Some(&yaml_min::Node::Null));
}

// --- yaml: the adversarial edges --------------------------------------------

/// A nested flow mapping recurses, and the bound on that recursion is the
/// one `MAX_DEPTH` promises. Before this was guarded, `{k: {k: ...}}` about
/// ten thousand deep — a 50 KB file — overflowed the stack and aborted the
/// process. That is the worst outcome a parser can have and it must stay
/// impossible.
#[test]
fn yaml_deep_flow_nesting_is_refused_not_overflowed() {
    let at = |n: usize| format!("a: {}1{}\n", "{k: ".repeat(n), "}".repeat(n));
    assert!(
        bounded("flow nesting at 128", move || yaml_min::parse(&at(128))).is_ok(),
        "the documented depth must still be accepted"
    );
    for n in [129usize, 10_000, 200_000] {
        let src = at(n);
        let err = bounded(&format!("flow nesting at {}", n), move || {
            yaml_min::parse(&src).expect_err("deep flow nesting must be refused")
        });
        assert!(err.contains("nesting too deep"), "at {}: {}", n, err);
    }
}

#[test]
fn yaml_deep_block_nesting_is_refused_not_overflowed() {
    let mut src = String::new();
    for i in 0..4000 {
        src.push_str(&" ".repeat(i));
        src.push_str(&format!("k{}:\n", i));
    }
    let err = bounded("block nesting", move || {
        yaml_min::parse(&src).expect_err("deep block nesting must be refused")
    });
    assert!(err.contains("nesting too deep"), "{}", err);
}

/// A long line and a long sequence must both terminate. 40 000 `- key:`
/// items is well past anything a compose file or manifest contains, and is
/// here because the reader rewrites its line list as it walks a sequence of
/// mappings — that work is quadratic, so this is the test that would notice
/// it becoming slow enough to look like a hang.
#[test]
fn yaml_long_lines_and_long_sequences_terminate() {
    let long = format!("a: {}\n", "x".repeat(1_000_000));
    assert!(bounded("1 MB line", move || yaml_min::parse(&long)).is_ok());

    let mut many = String::from("items:\n");
    for i in 0..40_000 {
        many.push_str(&format!("  - name: a{}\n", i));
    }
    let n = bounded("40k mapping items", move || yaml_min::parse(&many)).unwrap();
    assert_eq!(n.get("items").and_then(|v| v.as_seq()).map(<[_]>::len), Some(40_000));
}

#[test]
fn yaml_refuses_the_constructs_it_does_not_implement() {
    let cases = [
        ("a: &x 1\nb: *x\n", "anchors, aliases and tags are not supported"),
        ("a: !!str 1\n", "anchors, aliases and tags are not supported"),
        ("a: 1\n---\nb: 2\n", "multi-document YAML; use parse_all for streams"),
        ("- - x\n", "nested inline sequences (`- - x`) are not supported"),
    ];
    for (src, want) in cases {
        let err = yaml_min::parse(src).expect_err(src);
        assert!(err.contains(want), "{:?} gave {:?}", src, err);
    }
}

/// FINDING. An unterminated double-quoted scalar is accepted, and the
/// opening quote becomes part of the value. A customer whose manifest has
/// `image: "registry.example.com/acme/api:1.4.2` (one quote lost to an editor) gets an
/// image name beginning with a quote rather than a diagnostic — the exact
/// shape of defect this suite is looking for, pinned here so a future
/// change to it is deliberate.
#[test]
fn yaml_keeps_an_unterminated_quote_as_part_of_the_value() {
    let n = yaml_min::parse("image: \"registry.example.com/acme/api:1.4.2\nregion: eu-central-1\n").unwrap();
    assert_eq!(n.get("image").and_then(|v| v.as_str()), Some("\"registry.example.com/acme/api:1.4.2"));
    assert_eq!(n.get("region").and_then(|v| v.as_str()), Some("eu-central-1"));
}

/// FINDING. An unterminated flow sequence becomes a plain scalar holding
/// the bracket, rather than an error. `ports: [8080:8080` reads as the text
/// `[8080:8080`, so a dependent importer sees no sequence at all.
#[test]
fn yaml_keeps_an_unterminated_flow_sequence_as_a_scalar() {
    let n = yaml_min::parse("ports: [8080:8080, 443\n").unwrap();
    assert_eq!(n.get("ports").and_then(|v| v.as_str()), Some("[8080:8080, 443"));
    assert!(n.get("ports").and_then(|v| v.as_seq()).is_none());
}

/// FINDING. On a duplicate key this reader keeps both entries and `get`
/// returns the *first*. YAML 1.2 calls a duplicate key an error, and every
/// mainstream reader — including the one Docker and Kubernetes use — takes
/// the *last*. So a compose file that sets `image:` twice imports the
/// superseded image, silently. See the report: this needs a decision
/// (refuse, or take the last) rather than a quiet fix.
#[test]
fn yaml_picks_the_first_of_two_duplicate_keys() {
    let n = yaml_min::parse("image: first\nimage: second\n").unwrap();
    assert_eq!(n.get("image").and_then(|v| v.as_str()), Some("first"));
    assert_eq!(n.as_map().map(<[_]>::len), Some(2), "both entries are kept in the map");

    let out = ok(
        "compose",
        compose::import("name: x\nservices:\n  api:\n    image: a:OLD\n    image: a:NEW\n"),
    );
    assert_eq!(out.bwk.matches("image = \"a:OLD\"").count(), 1);
    assert_eq!(out.bwk.matches("image = \"a:NEW\"").count(), 0);
}

/// FINDING. A UTF-8 byte order mark is folded into the first key, so
/// `services:` on line 1 of a file saved by a Windows editor becomes a key
/// nobody looks for. The three readers disagree about this: HCL refuses the
/// mark, YAML and TOML absorb it.
#[test]
fn yaml_and_toml_fold_a_byte_order_mark_into_the_first_key() {
    let n = yaml_min::parse("\u{feff}services:\n  web:\n    image: nginx\n").unwrap();
    assert!(n.get("services").is_none(), "the key a reader would look for is not there");
    assert!(n.get("\u{feff}services").is_some(), "it is hiding behind the mark");

    let v = toml_min::parse("\u{feff}app = \"storefront\"\n").unwrap();
    assert!(v.get("app").is_none());
    assert!(v.get("\u{feff}app").is_some());

    // What the customer sees: compose refuses a file that does have
    // `services:`, and fly loses the app name without a word about it.
    let err = compose::import("\u{feff}services:\n  web:\n    image: nginx\n").unwrap_err();
    assert!(err.contains("no `services:` mapping"), "{}", err);
    let out = ok("fly", fly::import("\u{feff}app = \"storefront\"\nprimary_region = \"fra\"\n"));
    assert!(out.bwk.contains("workload app {"), "the app name is lost\n{}", out.bwk);
    assert!(out.notes.is_empty(), "and nothing is said about it: {:?}", out.notes);
}

/// A flow collection nested inside another is read as the nesting it is.
/// The splitter tracks brackets, so the comma in
/// `{requests: {cpu: 2, memory: 8Gi}}` belongs to the inner mapping, and
/// the two spellings of one manifest import to the same sizing.
///
/// Before this, the flow spelling produced **no `resources` line at all**,
/// with no note and no error: the customer's compute floor vanished and the
/// constraint price was computed against a workload that declared no floor.
#[test]
fn yaml_reads_a_nested_flow_collection_as_the_nesting_it_is() {
    let n = yaml_min::parse("resources: {requests: {cpu: 2, memory: 8Gi}}").unwrap();
    let req = n.get("resources").and_then(|r| r.get("requests")).expect("requests");
    assert_eq!(req.get("cpu").and_then(|v| v.as_str()), Some("2"));
    assert_eq!(req.get("memory").and_then(|v| v.as_str()), Some("8Gi"));

    let s = yaml_min::parse("a: [x, [y, z]]").unwrap();
    let outer = s.get("a").and_then(|v| v.as_seq()).expect("seq");
    assert_eq!(outer.len(), 2);
    assert_eq!(outer[0].as_str(), Some("x"));
    assert_eq!(outer[1].as_seq().map(<[_]>::len), Some(2));

    let e = yaml_min::parse("env: [{name: A, value: B}]").unwrap();
    let items = e.get("env").and_then(|v| v.as_seq()).expect("seq");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("name").and_then(|v| v.as_str()), Some("A"));
    assert_eq!(items[0].get("value").and_then(|v| v.as_str()), Some("B"));

    // The same manifest in both spellings must import identically.
    let flow = ok("k8s flow", k8s::import(FLOW_SIZED_POD));
    let block = ok("k8s block", k8s::import(BLOCK_SIZED_POD));
    assert_eq!(flow.bwk, block.bwk, "flow and block sizing must agree");
    assert_eq!(
        flow.bwk.matches("resources { vcpu >= 2  mem >= 8 GB }").count(),
        1,
        "{}",
        flow.bwk
    );
}

/// An unbalanced closer must not stop the split. The depth counter
/// saturates at zero, so a stray `}` cannot wrap it round and swallow every
/// comma after it — which would turn the rest of the collection into one
/// scalar without saying anything.
#[test]
fn yaml_keeps_splitting_after_an_unbalanced_closer() {
    let n = yaml_min::parse("a: [}, x, y]").unwrap();
    let a = n.get("a").and_then(|v| v.as_seq()).expect("seq");
    assert_eq!(a.len(), 3, "{:?}", a);
    assert_eq!(a[2].as_str(), Some("y"));
}

/// A comma inside quotes is still not a separator, at either nesting.
#[test]
fn yaml_keeps_a_comma_inside_quotes_out_of_the_split() {
    let n = yaml_min::parse("a: [\"q,uoted\", b]\nb: {k: \"v,w\"}\n").unwrap();
    let a = n.get("a").and_then(|v| v.as_seq()).expect("seq");
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].as_str(), Some("q,uoted"));
    assert_eq!(n.get("b").and_then(|v| v.get("k")).and_then(|v| v.as_str()), Some("v,w"));
}

/// An empty or explicitly null flow entry reads as null, not as text.
/// Flow-sequence items now go through the same reader as flow-mapping
/// values, which is where this comes from: `[a, , b]` used to hold an empty
/// scalar. Every importer reads these with `as_str`, so a null entry is
/// skipped rather than becoming a service named "".
#[test]
fn yaml_reads_an_empty_flow_entry_as_null() {
    let n = yaml_min::parse("a: [x, , y]\nb: [p, null]\nc: [q, ~]\n").unwrap();
    let a = n.get("a").and_then(|v| v.as_seq()).expect("seq");
    assert_eq!(a.len(), 3);
    assert_eq!(a[1], yaml_min::Node::Null);
    assert_eq!(n.get("b").and_then(|v| v.as_seq()).expect("seq")[1], yaml_min::Node::Null);
    assert_eq!(n.get("c").and_then(|v| v.as_seq()).expect("seq")[1], yaml_min::Node::Null);
}

/// The flow *sequence* branch recurses too, now that a nested sequence is
/// read as a sequence, so it needs the same bound the mapping branch has.
#[test]
fn yaml_deep_flow_sequence_nesting_is_refused_not_overflowed() {
    let at = |n: usize| format!("a: {}1{}\n", "[".repeat(n), "]".repeat(n));
    assert!(
        bounded("flow seq at 128", move || yaml_min::parse(&at(128))).is_ok(),
        "the documented depth must still be accepted"
    );
    for n in [129usize, 20_000, 200_000] {
        let src = at(n);
        let err = bounded(&format!("flow seq at {}", n), move || {
            yaml_min::parse(&src).expect_err("deep flow sequence nesting must be refused")
        });
        assert!(err.contains("nesting too deep"), "at {}: {}", n, err);
    }
}

/// Reading a nested flow collection costs one pass over the rest of the
/// line per level, so the work is bounded by `MAX_DEPTH` times the length
/// of the line — linear in each, with a 128x constant on the worst input.
/// 128 levels around a 100 KB payload is the shape to watch. This is here
/// so that a change making the factor worse than linear shows up as a
/// timeout rather than as a slow import nobody looked at.
#[test]
fn yaml_deep_and_long_flow_nesting_stays_bounded() {
    let src = format!("a: {}{}{}\n", "[".repeat(128), "x".repeat(100_000), "]".repeat(128));
    let n = bounded("128 levels around 100 KB", move || yaml_min::parse(&src)).unwrap();
    let mut node = n.get("a").expect("a");
    let mut levels = 0;
    while let Some(items) = node.as_seq() {
        assert_eq!(items.len(), 1, "one item at each level");
        levels += 1;
        node = &items[0];
    }
    assert_eq!(levels, 128, "every level must be read as a sequence");
    assert_eq!(node.as_str().map(str::len), Some(100_000));
}

/// A flat flow collection stays flat — including a flow `nodeSelector`,
/// where getting the split wrong would mis-state residency.
#[test]
fn yaml_reads_a_single_level_flow_collection() {
    let n = yaml_min::parse("a: [1, 2]\nb: {k: 1, j: 2}\nc: []\nd: {}\n").unwrap();
    assert_eq!(n.get("a").and_then(|v| v.as_seq()).map(<[_]>::len), Some(2));
    assert_eq!(n.get("b").and_then(|v| v.get("j")).and_then(|v| v.as_str()), Some("2"));
    assert_eq!(n.get("c").and_then(|v| v.as_seq()).map(<[_]>::len), Some(0));

    let out = ok("k8s", k8s::import(FLOW_NODE_SELECTOR));
    let res: Vec<(&str, &str)> =
        inferred(&out.bwk).into_iter().filter(|(f, _)| *f == "residency").collect();
    assert_eq!(res.len(), 1, "{}", out.bwk);
    assert!(res[0].1.contains("= us "), "{}", res[0].1);
    assert_eq!(cited_evidence(res[0].1), "us-east-1");
}

/// FINDING. `parse_all` splits on any line that is exactly `---`, including
/// one inside a literal block scalar — so a manifest holding a shell script
/// or an embedded YAML document is cut in the wrong place. Here the
/// Deployment loses everything after the `---`, which is its `resources`
/// and its `nodeSelector`, and the diagnostic blames the input's
/// indentation for something the reader got wrong.
#[test]
fn yaml_treats_a_separator_inside_a_block_scalar_as_a_separator() {
    let err =
        k8s::import(SEPARATOR_IN_BLOCK_SCALAR).expect_err("the split leaves an unparsable tail");
    assert!(err.contains("unexpected content"), "{}", err);
    assert!(
        !err.contains("block scalar"),
        "the diagnostic does not mention the real cause: {}",
        err
    );

    // A separator with a trailing comment is not recognised as one either.
    let err = yaml_min::parse_all("a: 1\n--- # second\nb: 2\n").expect_err("not split");
    assert!(err.contains("use parse_all"), "{}", err);
}

#[test]
fn yaml_parse_all_keeps_line_numbers_absolute() {
    let err = yaml_min::parse_all("a: 1\n---\nb: 2\n---\nc:\n\td: 3\n").expect_err("tab");
    assert!(err.starts_with("line 6:"), "the line must be the one in the file: {}", err);
}

// --- toml: the adversarial edges --------------------------------------------

#[test]
fn toml_refuses_every_unterminated_form() {
    let cases = [
        ("app = \"open", "unterminated string"),
        ("app = 'open", "unterminated literal string"),
        ("a = [1, 2", "unterminated array or inline table"),
        ("a = { b = 1", "unterminated array or inline table"),
        ("a = 1]", "unbalanced brackets"),
    ];
    for (src, want) in cases {
        let err = toml_min::parse(src).expect_err(src);
        assert!(err.contains(want), "{:?} gave {:?}", src, err);
    }
}

/// FINDING. An integer too large for `i64` is refused, but a float too
/// large for `f64` parses as infinity and is accepted. The quantity
/// parsers now refuse a non-finite value so it cannot reach the generated
/// BWK, but the reader itself still hands one out.
#[test]
fn toml_refuses_an_integer_overflow_but_accepts_a_float_one() {
    let err = toml_min::parse("x = 99999999999999999999999999").unwrap_err();
    assert!(err.contains("invalid number"), "{}", err);

    let v = toml_min::parse("x = 1e400").unwrap();
    match v.get("x") {
        Some(toml_min::Value::Float(f)) => assert!(f.is_infinite(), "{}", f),
        other => panic!("expected an infinite float, got {:?}", other),
    }
}

/// FINDING. Multi-line strings are refused, which the module promises, but
/// the diagnostic does not say so — it reports trailing characters, which
/// sends the reader looking at the wrong thing.
#[test]
fn toml_refuses_a_multiline_string_without_naming_it() {
    for src in ["x = \"\"\"\nhello\n\"\"\"\n", "x = '''\nhi\n'''\n"] {
        let err = toml_min::parse(src).expect_err(src);
        assert!(err.contains("unexpected trailing characters"), "{}", err);
        assert!(!err.contains("multi-line"), "the cause is still not named: {}", err);
    }
}

/// A duplicate key is refused. FINDING: a repeated `[table]` header is not
/// — the two tables merge silently, where TOML calls it an error.
#[test]
fn toml_refuses_a_duplicate_key_but_merges_a_repeated_table() {
    let err = toml_min::parse("[s]\nhost = \"a\"\nhost = \"b\"\n").unwrap_err();
    assert!(err.contains("duplicate key `host`"), "{}", err);

    let v = toml_min::parse("[s]\nhost = \"a\"\n[s]\nport = 1\n").unwrap();
    let s = v.get("s").expect("s");
    assert_eq!(s.get("host").and_then(|x| x.as_str()), Some("a"));
    assert_eq!(s.get("port").and_then(|x| x.as_i64()), Some(1));
}

#[test]
fn toml_deep_nesting_is_refused_not_overflowed() {
    let arrays = format!("x = {}{}", "[".repeat(50_000), "]".repeat(50_000));
    let err = bounded("toml arrays", move || toml_min::parse(&arrays).expect_err("deep"));
    assert!(err.contains("nesting too deep"), "{}", err);

    let tables = format!("x = {}1{}", "{ a = ".repeat(5_000), " }".repeat(5_000));
    let err = bounded("toml inline tables", move || toml_min::parse(&tables).expect_err("deep"));
    assert!(err.contains("nesting too deep"), "{}", err);

    // An unterminated bomb is caught before any recursion happens at all.
    let open = format!("x = {}", "[".repeat(100_000));
    let err = bounded("toml unterminated", move || toml_min::parse(&open).expect_err("open"));
    assert!(err.contains("unterminated array or inline table"), "{}", err);
}

#[test]
fn toml_reads_crlf_and_keeps_a_hash_inside_a_string() {
    let lf = toml_min::parse("app = \"x\"\n[s]\nk = 1\n").unwrap();
    let crlf = toml_min::parse("app = \"x\"\r\n[s]\r\nk = 1\r\n").unwrap();
    assert_eq!(lf, crlf);
    let v = toml_min::parse("a = \"x # kept\"\nb = 'y # kept'\nc = 1 # dropped\n").unwrap();
    assert_eq!(v.get("a").and_then(|x| x.as_str()), Some("x # kept"));
    assert_eq!(v.get("b").and_then(|x| x.as_str()), Some("y # kept"));
    assert_eq!(v.get("c").and_then(|x| x.as_i64()), Some(1));
}

#[test]
fn toml_refuses_an_unsupported_escape_and_a_bare_word() {
    assert!(toml_min::parse("a = \"\\q\"").unwrap_err().contains("unsupported escape"));
    assert!(toml_min::parse("a = maybe").unwrap_err().contains("unsupported value `maybe`"));
    assert!(toml_min::parse("a").unwrap_err().contains("expected `key = value`"));
    assert!(toml_min::parse("a.. = 1").unwrap_err().contains("empty key segment"));
}

// --- hcl: the adversarial edges ---------------------------------------------

/// HCL refuses a byte order mark rather than reading past it. That is the
/// right outcome and the opposite of what YAML and TOML do; the diagnostic
/// prints the mark, which is invisible, so it reads as an empty backtick
/// pair.
#[test]
fn hcl_refuses_a_byte_order_mark() {
    let err = hcl_min::parse("\u{feff}resource \"aws_instance\" \"a\" {\n}\n").unwrap_err();
    assert!(err.starts_with("line 1: unexpected character"), "{}", err);
    assert!(err.contains('\u{feff}'), "the mark itself is all the diagnostic shows: {:?}", err);
}

#[test]
fn hcl_refuses_every_unterminated_form() {
    let cases = [
        ("resource \"a\" \"b\" {\n x = 1\n", "expected `}`"),
        ("x = \"open", "unterminated string"),
        ("x = \"${open", "unterminated interpolation"),
        ("x = \"${foo(\"bar", "unterminated string"),
        ("x = <<EOF\nbody\n", "unterminated heredoc (missing `EOF`)"),
        ("x = [1, 2", "expected `,` or `]` in list"),
        ("x = [", "unterminated list"),
        ("/* open", "unterminated /* comment"),
        ("x = {\n a = 1\n", "unterminated object"),
        ("x = <<\nbody\n", "expected a heredoc marker after `<<`"),
    ];
    for (src, want) in cases {
        let err = hcl_min::parse(src).expect_err(src);
        assert!(err.contains(want), "{:?} gave {:?}", src, err);
    }
}

#[test]
fn hcl_deep_nesting_is_refused_not_overflowed() {
    let lists = format!("x = {}{}", "[".repeat(50_000), "]".repeat(50_000));
    let err = bounded("hcl lists", move || hcl_min::parse(&lists).expect_err("deep"));
    assert!(err.contains("nesting too deep"), "{}", err);

    let mut blocks = String::new();
    for _ in 0..5_000 {
        blocks.push_str("b {\n");
    }
    blocks.push_str(&"}\n".repeat(5_000));
    let err = bounded("hcl blocks", move || hcl_min::parse(&blocks).expect_err("deep"));
    assert!(err.contains("nesting too deep"), "{}", err);

    let interp = format!("x = \"{}\"", "${".repeat(50_000));
    let err = bounded("hcl interpolations", move || hcl_min::parse(&interp).expect_err("deep"));
    assert!(err.contains("unterminated"), "{}", err);

    let comment = format!("/*{}", "x".repeat(1_000_000));
    let err = bounded("hcl comment", move || hcl_min::parse(&comment).expect_err("open"));
    assert!(err.contains("unterminated /* comment"), "{}", err);
}

/// FINDING. An unterminated bracket inside a non-literal expression is
/// kept verbatim instead of refused, because such expressions are captured
/// as raw text by design. Low consequence — raw text is only ever scanned
/// for resource references — but it is not a diagnostic.
#[test]
fn hcl_keeps_an_unterminated_bracket_inside_a_raw_expression() {
    let body = hcl_min::parse("x = foo(1, 2\n").unwrap();
    assert_eq!(body.attr("x").and_then(|v| v.text()), Some("foo(1, 2"));
}

/// FINDING. A number too large for `f64` becomes infinity here too.
#[test]
fn hcl_accepts_a_number_overflow_as_infinity() {
    let body = hcl_min::parse("x = 1e400\n").unwrap();
    assert_eq!(body.attr("x").and_then(|v| v.as_f64()).map(f64::is_infinite), Some(true));
    assert!(hcl_min::parse("x = 1.2.3\n").unwrap_err().contains("invalid number"));
}

#[test]
fn hcl_reads_crlf_and_a_heredoc_marker_with_one() {
    let unix = hcl_min::parse("resource \"a\" \"b\" {\n  x = <<EOF\n  hi\n  EOF\n}\n").unwrap();
    let dos = hcl_min::parse("resource \"a\" \"b\" {\r\n  x = <<EOF\r\n  hi\r\n  EOF\r\n}\r\n")
        .expect("a Windows checkout must parse");
    assert_eq!(unix.blocks.len(), dos.blocks.len());
    assert_eq!(unix.blocks[0].body.attr("x").and_then(|v| v.as_str()), Some("  hi"));
}

// --- the importers: deliberate drops, on the record -------------------------

/// `count` and `for_each` still write one unit — multiplying the sizing
/// would invent a number — but the output now says what Terraform asked
/// for, that it was not applied, and what to do about it, the way the
/// Kubernetes importer treats `replicas`. Silence understated the estate by
/// exactly the multiplicity, and the constraint price with it.
#[test]
fn terraform_says_what_count_and_for_each_asked_for() {
    let cases = [
        ("count = 3", "count = 3 —"),
        (
            "count = length(var.subnets)",
            "count = length(var.subnets), which this importer does not evaluate —",
        ),
        ("for_each = { a = 1, b = 2 }", "for_each over 2 entries —"),
        (
            "for_each = toset([\"a\", \"b\"])",
            "for_each = toset([\"a\", \"b\"]), which this importer does not evaluate —",
        ),
        ("for_each = var.things", "for_each = var.things, which this importer does not evaluate —"),
    ];
    for (meta, want) in cases {
        let src = format!(
            "provider \"aws\" {{ region = \"eu-central-1\" }}\n\
             resource \"aws_instance\" \"api\" {{\n  {}\n  instance_type = \"m6i.2xlarge\"\n}}\n",
            meta
        );
        let out = ok("terraform", terraform::import(&src));
        assert_eq!(out.bwk.matches("  service ").count(), 1, "still one unit\n{}", out.bwk);
        assert_eq!(out.bwk.matches(want).count(), 1, "`{}` should say so\n{}", meta, out.bwk);
        assert_eq!(
            out.bwk.matches("what follows describes one of them, not the whole set.").count(),
            1,
            "{}",
            out.bwk
        );
        assert_eq!(out.bwk.matches("redundancy { vendors >= n }").count(), 1, "{}", out.bwk);
        assert_checks(meta, &out.bwk);
    }

    // A store says it on the data block as well, since the store is
    // understated by the same factor as the service that holds it.
    let store = ok(
        "terraform",
        terraform::import(
            "provider \"aws\" { region = \"eu-central-1\" }\n\
             resource \"aws_db_instance\" \"led\" {\n  count = 2\n  \
             instance_class = \"db.m6g.large\"\n}\n",
        ),
    );
    assert_eq!(
        store.bwk.matches("count = 2 —").count(),
        2,
        "on the data block and on the service\n{}",
        store.bwk
    );

    // A resource with neither says nothing, so the note is not noise.
    let plain = ok(
        "terraform",
        terraform::import(
            "provider \"aws\" { region = \"eu-central-1\" }\n\
             resource \"aws_instance\" \"api\" { instance_type = \"m6i.large\" }\n",
        ),
    );
    assert_eq!(plain.bwk.matches("not the whole set").count(), 0, "{}", plain.bwk);
}

/// FINDING. `extends` is not followed, so a service that inherits its image
/// and sizing imports with neither — and is labelled `built from source`,
/// which is not what the input says.
#[test]
fn compose_ignores_extends_and_mislabels_the_service() {
    let out = ok("compose", compose::import(COMPOSE_EXTENDS));
    assert_eq!(out.bwk.matches("  service api : backend-api {   // built from source").count(), 1);
    let api = out.bwk.split("service api").nth(1).unwrap();
    assert!(!api.contains("image ="), "no image was inherited\n{}", out.bwk);
    assert!(!api.contains("resources {"), "no sizing was inherited\n{}", out.bwk);
    assert!(out.notes.is_empty(), "and it is not reported: {:?}", out.notes);
}

/// FINDING. Only the first container of a pod is sized, so a sidecar's
/// request is dropped. Documented on the helper that does it, but silent
/// in the output: here 1.5 vCPU of request imports as 0.5.
#[test]
fn k8s_sizes_only_the_first_container_of_a_pod() {
    let out = ok("k8s", k8s::import(SIDECAR_POD));
    assert_eq!(out.bwk.matches("resources { vcpu >= 0.5  mem >= 0.5 GB }").count(), 1);
    assert_eq!(out.bwk.matches("envoy").count(), 0, "the sidecar is absent entirely");
    assert!(out.notes.is_empty(), "and it is not reported: {:?}", out.notes);
}

/// Residency is read from `nodeSelector` only, which the module states. A
/// customer who pins a region with `nodeAffinity` — the form Kubernetes
/// documents now — imports with no residency at all, so the residency
/// constraint prices at zero. Documented, and still worth the test.
#[test]
fn k8s_reads_node_selector_only_and_not_node_affinity() {
    let out = ok("k8s", k8s::import(NODE_AFFINITY_POD));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert!(out.notes.is_empty(), "nothing is said about the affinity: {:?}", out.notes);
}

/// FINDING. An unmapped `nodeSelector` region is reported for a Deployment
/// but not for a Job or CronJob, so the same input is explained in one
/// place and dropped in the other.
#[test]
fn k8s_reports_an_unmapped_region_for_a_deployment_but_not_for_a_cronjob() {
    let out = ok("k8s deployment", k8s::import(UNMAPPED_REGION_DEPLOYMENT));
    assert!(
        out.notes.iter().any(|n| n.contains("xx-nowhere-9") && n.contains("no BWK residency")),
        "{:?}",
        out.notes
    );

    let out = ok("k8s cronjob", k8s::import(UNMAPPED_REGION_CRONJOB));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert!(out.notes.is_empty(), "the same region goes unmentioned here: {:?}", out.notes);
}

#[test]
fn importers_report_what_they_ignored() {
    let out = ok("terraform", terraform::import(TF));
    assert!(
        out.notes.iter().any(|n| n.contains("ignored 1 resource(s)")
            && n.contains("aws_security_group.api")),
        "{:?}",
        out.notes
    );
    let out = ok("k8s", k8s::import(K8S));
    let note = out
        .notes
        .iter()
        .find(|n| n.contains("ignored 3 object(s)"))
        .unwrap_or_else(|| panic!("{:?}", out.notes));
    for want in ["Namespace shop", "Service api", "ConfigMap api-config"] {
        assert!(note.contains(want), "{} missing from {}", want, note);
    }
}

#[test]
fn importers_refuse_input_with_nothing_to_import() {
    assert!(compose::import("version: '3'\n").is_err());
    assert!(compose::import("services: {}\n").is_err());
    assert!(k8s::import("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\n").is_err());
    assert!(terraform::import("resource \"aws_security_group\" \"x\" { name = \"x\" }\n").is_err());
    assert!(k8s::import("").is_err());
}

// --- the importers: residency, which is the constraint that costs money ----

/// With no region anywhere — no resource attribute and no provider default
/// — the importer infers *nothing*. It must not guess a residency, because
/// a residency nobody declared is a constraint the customer would be
/// invoiced for.
#[test]
fn terraform_with_no_region_anywhere_infers_no_residency() {
    let src = "resource \"aws_instance\" \"api\" { instance_type = \"m6i.large\" }\n\
               resource \"aws_db_instance\" \"led\" { instance_class = \"db.m6g.large\" }\n";
    let out = ok("terraform", terraform::import(src));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert_eq!(out.bwk.matches("  service ").count(), 2, "{}", out.bwk);
    assert_eq!(out.bwk.matches("  data ").count(), 1, "{}", out.bwk);
    assert_checks("terraform no region", &out.bwk);
}

/// A region that is an expression is not a residency either: the importer
/// cannot see what `var.region` holds, so it says so instead of guessing.
#[test]
fn terraform_with_a_region_expression_infers_no_residency_and_says_why() {
    let src = "provider \"aws\" { region = var.region }\n\
               resource \"aws_instance\" \"api\" { instance_type = \"m6i.large\" }\n\
               resource \"aws_instance\" \"w\" {\n  instance_type = \"t3.small\"\n  \
               availability_zone = local.az\n}\n";
    let out = ok("terraform", terraform::import(src));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert!(
        out.notes.iter().any(|n| n.contains("var.region") && n.contains("manually")),
        "{:?}",
        out.notes
    );
    assert!(
        out.notes.iter().any(|n| n.contains("local.az") && n.contains("set it manually")),
        "{:?}",
        out.notes
    );
}

/// A region outside the residency regions BWK knows is left unconstrained
/// and reported, rather than rounded to the nearest one.
#[test]
fn an_unknown_region_is_left_unconstrained_and_reported() {
    let out = ok("fly", fly::import("app = \"x\"\nprimary_region = \"gru\"\n"));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert!(out.notes.iter().any(|n| n.contains("gru")), "{:?}", out.notes);

    let out = ok("fly", fly::import("app = \"x\"\n[http_service]\ninternal_port = 80\n"));
    assert!(out.notes.iter().any(|n| n.contains("no primary_region")), "{:?}", out.notes);

    let src = "provider \"aws\" { region = \"sa-east-1\" }\n\
               resource \"aws_instance\" \"api\" { instance_type = \"m6i.large\" }\n";
    let out = ok("terraform", terraform::import(src));
    assert_eq!(inferred(&out.bwk).iter().filter(|(f, _)| *f == "residency").count(), 0);
    assert!(out.notes.iter().any(|n| n.contains("sa-east-1")), "{:?}", out.notes);
}

/// A zone is reduced to its region before lookup, and the evidence quotes
/// the zone the customer actually wrote — `eu-central-1a`, not the
/// `eu-central-1` the lookup used.
#[test]
fn a_zone_is_reduced_to_its_region_and_the_evidence_quotes_the_zone() {
    let src = "resource \"aws_db_instance\" \"ledger\" {\n  instance_class = \"db.m6g.large\"\n  \
               availability_zone = \"eu-central-1a\"\n}\n";
    let out = ok("terraform", terraform::import(src));
    let res: Vec<(&str, &str)> =
        inferred(&out.bwk).into_iter().filter(|(f, _)| *f == "residency").collect();
    assert_eq!(res.len(), 2, "one on the data block, one on the service\n{}", out.bwk);
    for (_, line) in res {
        assert_eq!(cited_evidence(line), "eu-central-1a");
        assert!(line.contains("= eu "), "{}", line);
    }
}

/// A resource with no region of its own inherits the provider's, and the
/// evidence says which. A bare `from eu-central-1` on a resource that
/// carries no region sends the reader looking for a string that is not
/// there, and a marker whose whole job is to carry evidence has to carry
/// the right evidence.
#[test]
fn a_provider_default_region_is_cited_as_a_provider_default() {
    let out = ok("terraform", terraform::import(TF));
    // Five of the six units take the provider default; only the GCP
    // instance carries a zone of its own.
    assert_eq!(
        out.bwk.matches("// TODO: confirm; from provider \"aws\" default eu-central-1").count(),
        5,
        "{}",
        out.bwk
    );
    assert_eq!(out.bwk.matches("// TODO: confirm; from europe-west3-a").count(), 1, "{}", out.bwk);
    // Nothing cites a bare region that the resource does not carry.
    assert_eq!(out.bwk.matches("confirm; from eu-central-1").count(), 0, "{}", out.bwk);
}

/// An Azure location is matched exactly rather than by prefix, so the
/// spaces in `West Europe` — the spelling the portal uses — have to come
/// out before the lookup. Without that, the location yields no residency at
/// all. (The AWS and Google zone reductions in the same function cannot
/// change an answer, because those lookups are by prefix; see the report.)
#[test]
fn an_azure_location_written_with_spaces_still_yields_a_residency() {
    let src = "provider \"azurerm\" { }\n\
               resource \"azurerm_linux_virtual_machine\" \"api\" {\n  \
               location = \"West Europe\"\n  size = \"Standard_D2s_v5\"\n}\n";
    let out = ok("terraform", terraform::import(src));
    let res: Vec<(&str, &str)> =
        inferred(&out.bwk).into_iter().filter(|(f, _)| *f == "residency").collect();
    assert_eq!(res.len(), 1, "{}", out.bwk);
    assert!(res[0].1.contains("// TODO: confirm; from West Europe"), "{}", res[0].1);
    assert!(res[0].1.contains("= eu "), "{}", res[0].1);
}

/// A reference to one resource is not a reference to another whose name
/// merely begins the same way. Before this was fixed, matching
/// `aws_db_instance.main` inside `aws_db_instance.main_replica` gave a
/// service a data dependency it does not have.
#[test]
fn terraform_does_not_confuse_a_resource_with_a_longer_named_sibling() {
    let src = "provider \"aws\" { region = \"eu-central-1\" }\n\
               resource \"aws_db_instance\" \"main\" { instance_class = \"db.t3.medium\" }\n\
               resource \"aws_db_instance\" \"main_replica\" { instance_class = \"db.t3.medium\" }\n\
               resource \"aws_instance\" \"reader\" {\n  instance_type = \"m6i.large\"\n  \
               user_data = \"host=${aws_db_instance.main_replica.address}\"\n}\n";
    let out = ok("terraform", terraform::import(src));
    let reader = out.bwk.split("service reader").nth(1).unwrap();
    let reader = reader.split("\n  }").next().unwrap();
    assert_eq!(reader.matches("depends_on = main_replica-store").count(), 1, "{}", reader);
    assert_eq!(reader.matches("depends_on = main-store").count(), 0, "{}", reader);
}

/// The reference the importer *should* find is still found, including one
/// buried in a heredoc and one in an explicit `depends_on` list.
#[test]
fn terraform_finds_a_reference_in_a_heredoc_and_in_depends_on() {
    let out = ok("terraform", terraform::import(TF));
    let api = out.bwk.split("service api").nth(1).unwrap().split("\n  }").next().unwrap();
    assert_eq!(api.matches("depends_on = main-store").count(), 1, "from the heredoc\n{}", api);
    let worker = out.bwk.split("service worker").nth(1).unwrap().split("\n  }").next().unwrap();
    assert_eq!(worker.matches("depends_on = main-store").count(), 1, "{}", worker);
    assert_eq!(worker.matches("depends_on = media").count(), 1, "{}", worker);
}

// --- the importers: names and quantities must keep the output checkable ----

/// Two objects can reduce to one BWK name. A workload cannot hold two data
/// blocks under one name, so the output would not check; the clash is
/// resolved and reported rather than written out broken.
#[test]
fn k8s_keeps_data_block_names_unique_and_reports_the_clash() {
    let out = ok("k8s", k8s::import(CLASHING_NAMES));
    assert_eq!(out.bwk.matches("  data db-store {").count(), 1, "{}", out.bwk);
    assert_checks("k8s name clash", &out.bwk);
    assert!(
        out.notes.iter().any(|n| n.contains("both become the data block `db-store`")),
        "{:?}",
        out.notes
    );
    assert_eq!(out.bwk.matches("depends_on = db-store").count(), 1, "{}", out.bwk);
}

/// Terraform resolves its own name clashes by suffixing, so two resources
/// of different types sharing a name both survive.
#[test]
fn terraform_keeps_service_names_unique() {
    let src = "provider \"aws\" { region = \"eu-central-1\" }\n\
               resource \"aws_instance\" \"api\" { instance_type = \"m6i.large\" }\n\
               resource \"aws_lambda_function\" \"api\" { }\n";
    let out = ok("terraform", terraform::import(src));
    assert_eq!(out.bwk.matches("  service api :").count(), 1, "{}", out.bwk);
    assert_eq!(out.bwk.matches("  service api-2 :").count(), 1, "{}", out.bwk);
    assert_checks("terraform name clash", &out.bwk);
}

/// A quantity that is not a finite number is read as no quantity at all. A
/// literal too large for `f64` parses as infinity, and `vcpu >= inf` does
/// not check — the generated BWK would be unusable, which the importers
/// promise never to produce.
#[test]
fn a_quantity_that_is_not_finite_never_reaches_the_output() {
    let huge = overflowing();
    assert!(huge.parse::<f64>().unwrap().is_infinite(), "the fixture must actually overflow");

    let out = ok("compose", compose::import(&COMPOSE_OVERFLOW.replace("NUM", &huge)));
    assert_eq!(out.bwk.matches("resources {").count(), 0, "{}", out.bwk);
    assert_checks("compose overflow", &out.bwk);

    let out = ok("k8s", k8s::import(&K8S_OVERFLOW.replace("NUM", &huge)));
    // Asserted on the assignments, not on the text: the "set a deadline"
    // suggestion the importer leaves behind is a comment that also reads
    // `deadline = 6 h`, and a bare substring search would match it.
    let fields: Vec<&str> = assignments(&out.bwk).into_iter().map(|(f, _)| f).collect();
    assert!(!fields.contains(&"deadline"), "{}", out.bwk);
    assert_eq!(out.bwk.matches("gpu >=").count(), 0, "{}", out.bwk);
    assert_checks("k8s overflow", &out.bwk);

    // A finite quantity still arrives, so the guard is not simply dropping
    // every number.
    let out = ok("compose", compose::import(COMPOSE_SIZED));
    assert_eq!(out.bwk.matches("resources { vcpu >= 2  mem >= 2 GB }").count(), 1, "{}", out.bwk);
}

/// A name that is not a BWK identifier is rewritten, and the rewrite is
/// reported so the customer can see which of their names changed.
#[test]
fn a_name_that_is_not_an_identifier_is_rewritten_and_reported() {
    let out = ok("fly", fly::import("app = \"my.app\"\nprimary_region = \"fra\"\n"));
    assert_eq!(out.bwk.matches("workload my-app {").count(), 1, "{}", out.bwk);
    assert!(
        out.notes.iter().any(|n| n.contains("`my.app`") && n.contains("`my-app`")),
        "{:?}",
        out.notes
    );
    assert_checks("fly rename", &out.bwk);

    let out = ok("compose", compose::import("services:\n  my.svc:\n    image: a:1\n"));
    assert!(out.notes.iter().any(|n| n.contains("`my.svc`")), "{:?}", out.notes);
    assert_checks("compose rename", &out.bwk);
}

// --- what cannot pass without changing what a parse means ------------------

/// A duplicate key should not be resolved by quietly taking one of them.
///
/// To pass: `parse_map` must refuse a repeated key, or `Node::get` must
/// return the last. Refusing matches what this reader already does with
/// anchors and tags — say so rather than guess — and matches YAML 1.2.
/// Taking the last matches Docker and Kubernetes. Either is a change to
/// what a parse means, so it is a decision, not a fix.
#[test]
#[ignore = "needs a decision: refuse a duplicate key, or take the last"]
fn yaml_should_not_quietly_pick_one_of_two_duplicate_keys() {
    let r = yaml_min::parse("image: first\nimage: second\n");
    match r {
        Err(e) => assert!(e.contains("duplicate key"), "{}", e),
        Ok(n) => assert_eq!(n.get("image").and_then(|v| v.as_str()), Some("second")),
    }
}

/// A byte order mark should not become part of a key.
///
/// To pass: `yaml_min::parse`, `yaml_min::parse_all` and `toml_min::parse`
/// must drop a leading `\u{feff}` before tokenising. HCL does not need it —
/// it refuses the mark already, which is also acceptable. This is a change
/// to what input is accepted, so it is a decision.
#[test]
#[ignore = "needs a decision: strip a leading byte order mark, or refuse it"]
fn yaml_and_toml_should_not_absorb_a_byte_order_mark() {
    let n = yaml_min::parse("\u{feff}services:\n  web:\n    image: nginx\n").unwrap();
    assert!(n.get("services").is_some());
    let v = toml_min::parse("\u{feff}app = \"storefront\"\n").unwrap();
    assert_eq!(v.get("app").and_then(|x| x.as_str()), Some("storefront"));
}

/// A `---` inside a block scalar is content, not a document separator.
///
/// To pass: `parse_all` must know where block scalars begin and end before
/// it splits, which means tracking indentation while scanning — the split
/// is currently a line-by-line pass that knows nothing about structure.
/// That is a rewrite of the splitter rather than a small fix.
#[test]
#[ignore = "parse_all splits before it knows about block scalars; see the report"]
fn yaml_parse_all_should_not_split_inside_a_block_scalar() {
    let src = "kind: ConfigMap\ndata:\n  run.sh: |\n    echo one\n    ---\n    echo two\n";
    let docs = yaml_min::parse_all(src).unwrap();
    assert_eq!(docs.len(), 1);
    let sh = docs[0].get("data").and_then(|d| d.get("run.sh")).and_then(|v| v.as_str());
    assert_eq!(sh, Some("echo one\n---\necho two"));
}

/// A refused multi-line string should name itself.
///
/// To pass: `toml_min` must notice `"""` or `'''` and say so, instead of
/// reporting trailing characters after an empty string. The module promises
/// "a clear error"; this would be one. It is a new diagnostic rather than a
/// behaviour change, but the message is part of the published surface.
#[test]
#[ignore = "would add a diagnostic to a published crate; see the report"]
fn toml_should_name_a_multiline_string_in_its_error() {
    let err = toml_min::parse("x = \"\"\"\nhello\n\"\"\"\n").unwrap_err();
    assert!(err.contains("multi-line"), "{}", err);
}
