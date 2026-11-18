//! The language server: its pure functions (diagnostics, completion,
//! hover, symbols, definition, formatting) and one real JSON-RPC session
//! against the built binary over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use bailiwick::json::{self, Json};
use bailiwick::lsp::{self, Ctx};

const STOREFRONT: &str = include_str!("../examples/storefront.bwk");

// --- pure functions ------------------------------------------------------------

#[test]
fn diagnostics_have_ranges_on_the_offending_token() {
    let text = "workload w {\n  service s : backend-api {\n    slo.p95 <= $5\n  }\n}\n";
    let diags = lsp::diagnostics("untitled:1", text);
    assert_eq!(diags.len(), 1, "{:?}", diags);
    let d = &diags[0];
    assert_eq!(d.get("severity").and_then(|s| s.as_f64()), Some(1.0));
    assert!(d.get("message").and_then(|m| m.as_str()).unwrap().contains("duration"));
    let start = d.get("range").and_then(|r| r.get("start")).unwrap();
    assert_eq!(start.get("line").and_then(|l| l.as_f64()), Some(2.0));
    assert_eq!(start.get("character").and_then(|c| c.as_f64()), Some(15.0), "points at the `$`");
}

#[test]
fn diagnostics_resolve_imports_for_file_uris() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/storefront-eu-policy.bwk");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = lsp::path_to_uri(&path);
    assert!(lsp::diagnostics(&uri, &text).is_empty(), "the example is clean");
    // Break the import and the loader reports it.
    let broken = text.replace("policies/eu-only.bwk", "policies/missing.bwk");
    let diags = lsp::diagnostics(&uri, &broken);
    assert!(diags.iter().any(|d| d.get("message").and_then(|m| m.as_str()).unwrap().contains("cannot read")));
}

#[test]
fn context_follows_braces() {
    let text = "workload w {\n  service s : backend-api {\n    resources { vcpu >= 2 }\n    \n  }\n  \n}\n";
    assert_eq!(lsp::context_at(text, 0, 0), Ctx::TopLevel);
    assert_eq!(lsp::context_at(text, 1, 2), Ctx::Block("workload".into()));
    assert_eq!(lsp::context_at(text, 3, 4), Ctx::Block("service".into()));
    assert_eq!(lsp::context_at(text, 2, 16), Ctx::Block("resources".into()));
    assert_eq!(lsp::context_at(text, 5, 2), Ctx::Block("workload".into()));
    assert_eq!(lsp::context_at(text, 7, 0), Ctx::TopLevel);
}

fn labels(items: &[Json]) -> Vec<String> {
    items.iter().filter_map(|i| i.get("label").and_then(|l| l.as_str()).map(|s| s.to_string())).collect()
}

#[test]
fn completions_are_context_aware() {
    let text = "workload w {\n  \n  service s : backend-api {\n    residency = \n    \n  }\n}\n";
    let top = labels(&lsp::completions("", 0, 0));
    assert!(top.contains(&"workload".to_string()) && top.contains(&"policy".to_string()));
    let in_workload = labels(&lsp::completions(text, 1, 2));
    assert!(in_workload.contains(&"service".to_string()) && !in_workload.contains(&"vcpu".to_string()));
    let in_service = labels(&lsp::completions(text, 4, 4));
    assert!(in_service.contains(&"slo.p95".to_string()) && in_service.contains(&"cache".to_string()));
    let regions = labels(&lsp::completions(text, 3, 16));
    assert!(regions.contains(&"eu".to_string()) && regions.contains(&"california".to_string()));
    let classes = labels(&lsp::completions("workload w {\n  service s : \n}\n", 1, 14));
    assert!(classes.contains(&"backend-api".to_string()) && !classes.contains(&"eu".to_string()));
    let levels = labels(&lsp::completions("workload w {\n  data d { sensitivity = \n}\n}\n", 1, 25));
    assert_eq!(levels, vec!["S0", "S1", "S2", "S3"]);
}

#[test]
fn hover_documents_levels_regions_and_keywords() {
    let text = STOREFRONT;
    // 0-based line 9: `    sensitivity = S3            // ...`
    let h = lsp::hover(text, 9, 18).expect("hover on S3");
    assert!(h.contains("sensitivity level 3") && h.contains("T3"), "{}", h);
    // line 10: `    residency   = eu`
    let h = lsp::hover(text, 10, 18).expect("hover on eu");
    assert!(h.contains("residency region") && h.contains("de"), "{}", h);
    let slo = "workload w {\n  service s : backend-api {\n    slo.p95 <= 200 ms from eu\n  }\n}\n";
    let h = lsp::hover(slo, 2, 4).expect("hover on slo");
    assert!(h.contains("latency target") && h.contains("measured"), "{}", h);
    // line 17: `    resources  { vcpu >= 2  mem >= 4 GB }` — a unit
    let h = lsp::hover(text, 17, 37).expect("hover on GB");
    assert!(h.contains("2^30"), "{}", h);
    assert!(lsp::hover(text, 0, 0).is_none(), "comments have no hover");
    assert!(lsp::doc_for("budget", &Ctx::Block("policy".into())).unwrap().contains("cap"));
    assert!(lsp::doc_for("budget", &Ctx::Block("workload".into())).unwrap().contains("the workload's budget"));
    assert!(lsp::doc_for("GB", &Ctx::TopLevel).unwrap().contains("2^30"));
}

#[test]
fn symbols_outline_the_document() {
    let syms = lsp::symbols(STOREFRONT);
    assert_eq!(syms.len(), 1);
    let w = &syms[0];
    assert_eq!(w.get("name").and_then(|n| n.as_str()), Some("storefront"));
    let children = w.get("children").and_then(|c| c.as_arr()).unwrap();
    let names: Vec<&str> = children.iter().map(|c| c.get("name").and_then(|n| n.as_str()).unwrap()).collect();
    assert_eq!(names, vec!["customer_db", "web", "media", "nightly_report"]);
    assert_eq!(children[1].get("detail").and_then(|d| d.as_str()), Some("service : backend-api"));
}

#[test]
fn definition_jumps_to_data_blocks_and_imports() {
    // 0-based line 18: `    depends_on  = customer_db   // ...`
    let def = lsp::definition("untitled:1", STOREFRONT, 18, 20).expect("definition");
    let start = def.get("range").and_then(|r| r.get("start")).unwrap();
    assert_eq!(start.get("line").and_then(|l| l.as_f64()), Some(8.0), "the data block opens on 0-based line 8");
    assert_eq!(start.get("character").and_then(|c| c.as_f64()), Some(7.0), "the name, not the keyword");

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/storefront-eu-policy.bwk");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = lsp::path_to_uri(&path);
    let import_line = text.lines().position(|l| l.starts_with("import")).unwrap();
    let def = lsp::definition(&uri, &text, import_line, 10).expect("import definition");
    assert!(def.get("uri").and_then(|u| u.as_str()).unwrap().ends_with("examples/policies/eu-only.bwk"));
}

#[test]
fn formatting_returns_one_whole_document_edit() {
    let messy = "workload w {\nprofile=balanced\n service s : backend-api {   slo.p95 <= 200 ms }\n}\n";
    let edits = lsp::format_edits(messy);
    assert_eq!(edits.len(), 1);
    let new_text = edits[0].get("newText").and_then(|t| t.as_str()).unwrap();
    assert!(new_text.contains("  profile = balanced\n"));
    assert!(lsp::format_edits(new_text).is_empty(), "already formatted → no edits");
    assert!(lsp::format_edits("workload {").is_empty(), "errors → no edits");
}

// --- a real session over stdio -------------------------------------------------------

fn send(stdin: &mut impl Write, body: &str) {
    write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    stdin.flush().unwrap();
}

fn recv(reader: &mut BufReader<impl Read>) -> Json {
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            if len > 0 {
                break;
            }
            continue;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).unwrap();
    json::parse(std::str::from_utf8(&buf).unwrap()).unwrap()
}

#[test]
fn stdio_session_initializes_diagnoses_completes_and_exits() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bailiwick"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn bailiwick lsp");
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());

    send(&mut stdin, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#);
    let init = recv(&mut out);
    assert_eq!(init.get("id").and_then(|i| i.as_f64()), Some(1.0));
    let caps = init.get("result").and_then(|r| r.get("capabilities")).expect("capabilities");
    assert_eq!(caps.get("hoverProvider").and_then(|h| h.as_bool()), Some(true));

    send(&mut stdin, r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);

    let doc = "workload w {\n  service s : backend-api {\n    slo.p95 <= $5\n  }\n}\n";
    let open = Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("textDocument/didOpen")),
        (
            "params",
            Json::obj(vec![(
                "textDocument",
                Json::obj(vec![
                    ("uri", Json::str("untitled:test.bwk")),
                    ("languageId", Json::str("bwk")),
                    ("version", Json::Num(1.0)),
                    ("text", Json::str(doc)),
                ]),
            )]),
        ),
    ]);
    send(&mut stdin, &json::to_string_compact(&open));
    let published = recv(&mut out);
    assert_eq!(published.get("method").and_then(|m| m.as_str()), Some("textDocument/publishDiagnostics"));
    let diags = published.get("params").and_then(|p| p.get("diagnostics")).and_then(|d| d.as_arr()).unwrap();
    assert_eq!(diags.len(), 1);

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"untitled:test.bwk"},"position":{"line":1,"character":2}}}"#,
    );
    let comp = recv(&mut out);
    let items = comp.get("result").and_then(|r| r.as_arr()).unwrap();
    assert!(labels(items).contains(&"service".to_string()));

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{"textDocument":{"uri":"untitled:test.bwk"},"position":{"line":2,"character":5}}}"#,
    );
    let hov = recv(&mut out);
    let value = hov.get("result").and_then(|r| r.get("contents")).and_then(|c| c.get("value")).and_then(|v| v.as_str()).unwrap();
    assert!(value.contains("latency target"), "{}", value);

    send(&mut stdin, r#"{"jsonrpc":"2.0","id":4,"method":"nonexistent/method","params":{}}"#);
    let err = recv(&mut out);
    assert_eq!(err.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_f64()), Some(-32601.0));

    send(&mut stdin, r#"{"jsonrpc":"2.0","id":5,"method":"shutdown"}"#);
    let sd = recv(&mut out);
    assert_eq!(sd.get("result"), Some(&Json::Null));
    send(&mut stdin, r#"{"jsonrpc":"2.0","method":"exit"}"#);
    let status = child.wait().unwrap();
    assert!(status.success(), "{:?}", status);
}
