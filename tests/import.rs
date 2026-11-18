//! Importers and the minimal TOML/YAML/HCL readers behind them. The
//! contract for every importer: the output is valid BWK that passes
//! `check`, and the structure (services, stores, dependencies, sizing,
//! residency hints) is recovered faithfully.

use bailiwick::import::{
    classify_image, compose, fly, hcl_min, k8s, parse_cpu, parse_mem_gb, sanitize_ident,
    terraform, toml_min, yaml_min,
};
use bailiwick::{any_errors, parse_and_check};

const FLY: &str = include_str!("../examples/import/fly.toml");
const COMPOSE: &str = include_str!("../examples/import/docker-compose.yml");
const K8S: &str = include_str!("../examples/import/k8s.yaml");
const TF: &str = include_str!("../examples/import/main.tf");

// --- toml -------------------------------------------------------------------

#[test]
fn toml_reads_fly_shape() {
    let v = toml_min::parse(FLY).expect("fly.toml should parse");
    assert_eq!(v.get("app").and_then(|x| x.as_str()), Some("storefront-api"));
    assert_eq!(v.get("primary_region").and_then(|x| x.as_str()), Some("fra"));
    let hs = v.get("http_service").expect("[http_service]");
    assert_eq!(hs.get("internal_port").and_then(|x| x.as_i64()), Some(8080));
    assert_eq!(hs.get("force_https").and_then(|x| x.as_bool()), Some(true));
    let mounts = v.get("mounts").and_then(|x| x.as_array()).expect("[[mounts]]");
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].get("source").and_then(|x| x.as_str()), Some("pgdata"));
    let vm = v.get("vm").and_then(|x| x.as_array()).expect("[[vm]]");
    assert_eq!(vm[0].get("cpus").and_then(|x| x.as_i64()), Some(2));
    let procs = hs.get("processes").and_then(|x| x.as_array()).expect("processes");
    assert_eq!(procs.len(), 2);
}

#[test]
fn toml_handles_dotted_keys_inline_tables_and_strings() {
    let src = r#"
        # comment
        title = "x # not a comment"
        a.b.c = 1
        point = { x = 1.5, y = -2 }
        tags = [ "one", 'two', ]
        [server]
        host = 'lit"eral'
        [[items]]
        name = "first"
        [items.meta]
        k = true
        [[items]]
        name = "second"
    "#;
    let v = toml_min::parse(src).unwrap();
    assert_eq!(v.get("title").and_then(|x| x.as_str()), Some("x # not a comment"));
    assert_eq!(
        v.get("a").and_then(|a| a.get("b")).and_then(|b| b.get("c")).and_then(|c| c.as_i64()),
        Some(1)
    );
    assert_eq!(v.get("point").and_then(|p| p.get("y")).and_then(|y| y.as_i64()), Some(-2));
    assert_eq!(v.get("tags").and_then(|t| t.as_array()).map(|a| a.len()), Some(2));
    assert_eq!(v.get("server").and_then(|s| s.get("host")).and_then(|h| h.as_str()), Some("lit\"eral"));
    let items = v.get("items").and_then(|i| i.as_array()).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("meta").and_then(|m| m.get("k")).and_then(|k| k.as_bool()), Some(true));
    assert_eq!(items[1].get("name").and_then(|n| n.as_str()), Some("second"));
}

#[test]
fn toml_rejects_dates_clearly() {
    let err = toml_min::parse("when = 2026-01-01").unwrap_err();
    assert!(err.contains("dates"), "{}", err);
}

// --- yaml -------------------------------------------------------------------

#[test]
fn yaml_reads_compose_shape() {
    let n = yaml_min::parse(COMPOSE).expect("compose should parse");
    let services = n.get("services").and_then(|s| s.as_map()).expect("services map");
    let names: Vec<&str> = services.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, vec!["web", "api", "worker", "db", "cache"]);
    let api = n.get("services").unwrap().get("api").unwrap();
    assert_eq!(api.get("image").and_then(|i| i.as_str()), Some("registry.example.com/acme/api:1.4.2"));
    let ports = api.get("ports").and_then(|p| p.as_seq()).unwrap();
    assert_eq!(ports[0].as_str(), Some("8080:8080"));
    let deps = api.get("depends_on").and_then(|d| d.as_map()).unwrap();
    assert_eq!(deps[0].0, "db");
    let cmd = n.get("services").unwrap().get("worker").unwrap().get("command").unwrap();
    assert_eq!(cmd.as_seq().map(|s| s.len()), Some(3));
    let vols = n.get("volumes").and_then(|v| v.as_map()).unwrap();
    assert_eq!(vols[0].0, "pgdata");
}

#[test]
fn yaml_sequence_of_mappings_with_nested_maps() {
    // The second item's first key has nested content deeper than its
    // sibling keys — the shape of `volumeClaimTemplates` in a StatefulSet.
    let src = "items:\n  - name: a\n    size: 1\n  - metadata:\n      name: b\n    spec:\n      storage: 2\n";
    let n = yaml_min::parse(src).unwrap();
    let items = n.get("items").and_then(|i| i.as_seq()).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("size").and_then(|x| x.as_str()), Some("1"));
    assert_eq!(
        items[1].get("metadata").and_then(|m| m.get("name")).and_then(|x| x.as_str()),
        Some("b")
    );
    assert_eq!(
        items[1].get("spec").and_then(|s| s.get("storage")).and_then(|x| x.as_str()),
        Some("2")
    );
}

#[test]
fn yaml_block_scalars_parse() {
    let n = yaml_min::parse("a: |\n  line one\n  line two\nb: >\n  folded\n  text\nc: 1\n").unwrap();
    assert_eq!(n.get("a").and_then(|x| x.as_str()), Some("line one\nline two"));
    assert_eq!(n.get("b").and_then(|x| x.as_str()), Some("folded text"));
    assert_eq!(n.get("c").and_then(|x| x.as_str()), Some("1"));
}

#[test]
fn yaml_parse_all_splits_documents() {
    let docs = yaml_min::parse_all("---\na: 1\n---\n# only a comment\n---\nb: 2\n").unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[1].get("b").and_then(|x| x.as_str()), Some("2"));
    let docs = yaml_min::parse_all(K8S).unwrap();
    assert_eq!(docs.len(), 7);
}

#[test]
fn yaml_rejects_unsupported_constructs_clearly() {
    assert!(yaml_min::parse("a: &x 1\nb: *x\n").unwrap_err().contains("anchors"));
    assert!(yaml_min::parse("a:\n\t- x\n").unwrap_err().contains("tabs"));
    assert!(yaml_min::parse("a: 1\n---\nb: 2\n").unwrap_err().contains("parse_all"));
}

// --- hcl ----------------------------------------------------------------------

#[test]
fn hcl_reads_blocks_attributes_and_expressions() {
    let body = hcl_min::parse(TF).expect("main.tf should parse");
    let providers = body.blocks_of("provider");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].labels, vec!["aws".to_string()]);
    assert_eq!(providers[0].body.attr("region").and_then(|v| v.as_str()), Some("eu-central-1"));

    let resources = body.blocks_of("resource");
    assert_eq!(resources.len(), 6);
    let api = resources.iter().find(|b| b.labels[1] == "api" && b.labels[0] == "aws_instance").unwrap();
    // A reference is kept raw, not evaluated.
    assert_eq!(api.body.attr("ami").and_then(|v| v.text()), Some("data.aws_ami.ubuntu.id"));
    // A heredoc with an interpolation keeps the reference text.
    let ud = api.body.attr("user_data").and_then(|v| v.as_str()).unwrap();
    assert!(ud.contains("${aws_db_instance.main.address}"), "{}", ud);
    // Objects and lists.
    assert_eq!(api.body.attr("tags").and_then(|t| t.get("Team")).and_then(|v| v.as_str()), Some("shop"));
    let worker = resources.iter().find(|b| b.labels[1] == "worker").unwrap();
    let deps = worker.body.attr("depends_on").and_then(|v| v.as_list()).unwrap();
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0].text(), Some("aws_db_instance.main"));
    // Nested blocks and numbers.
    let db = resources.iter().find(|b| b.labels[0] == "aws_db_instance").unwrap();
    assert_eq!(db.body.attr("allocated_storage").and_then(|v| v.as_f64()), Some(50.0));
    let gcp = resources.iter().find(|b| b.labels[0] == "google_compute_instance").unwrap();
    assert!(gcp.body.block("boot_disk").and_then(|b| b.body.block("initialize_params")).is_some());
    let sg = resources.iter().find(|b| b.labels[0] == "aws_security_group").unwrap();
    let cidrs = sg.body.block("ingress").unwrap().body.attr("cidr_blocks").and_then(|v| v.as_list()).unwrap();
    assert_eq!(cidrs[0].as_str(), Some("0.0.0.0/0"));
}

#[test]
fn hcl_handles_comments_and_trailing_expressions() {
    let src = "a = 1 // trailing\n/* block\n comment */\nb = local.x # hash\nc = fn(1, [2, 3]) \nd = true\n";
    let body = hcl_min::parse(src).unwrap();
    assert_eq!(body.attr("a").and_then(|v| v.as_f64()), Some(1.0));
    assert_eq!(body.attr("b").and_then(|v| v.text()), Some("local.x"));
    assert_eq!(body.attr("c").and_then(|v| v.text()), Some("fn(1, [2, 3])"));
    assert_eq!(body.attr("d"), Some(&hcl_min::Value::Bool(true)));
}

#[test]
fn hcl_rejects_unterminated_input_clearly() {
    assert!(hcl_min::parse("resource \"a\" \"b\" {\n x = 1\n").unwrap_err().contains("expected `}`"));
    assert!(hcl_min::parse("x = \"open").unwrap_err().contains("unterminated string"));
}

// --- importers ----------------------------------------------------------------

#[test]
fn fly_import_yields_valid_awl_with_residency_mount_and_resources() {
    let out = fly::import(FLY).expect("fly import");
    let (file, diags) = parse_and_check(&out.bwk);
    assert!(!any_errors(&diags), "generated BWK must check: {:?}\n{}", diags, out.bwk);
    let w = &file.workloads[0];
    assert_eq!(w.name, "storefront-api");
    assert_eq!(w.data.len(), 1, "one [[mounts]] → one data block");
    assert_eq!(w.data[0].name, "pgdata");
    assert_eq!(w.data[0].residency.as_deref(), Some("eu"), "fra → eu");
    let svc = &w.services[0];
    assert_eq!(svc.residency.as_deref(), Some("eu"));
    assert_eq!(svc.depends_on, vec!["pgdata".to_string()]);
    assert_eq!(svc.image.as_deref(), Some("registry.example.com/acme/storefront-api:2.3.0"));
    assert_eq!(svc.resources.vcpu, Some(2.0), "[[vm]] cpus = 2");
    assert_eq!(svc.resources.mem_gb, Some(1.0), "[[vm]] memory = 1gb");
    // By count and by site. A bare `contains("TODO")` would also pass on the
    // commented-out `// slo.p95 ... // TODO: set a latency target` suggestion,
    // which is not a constraint, so it could not fail for the right reason.
    assert_eq!(out.bwk.matches("// TODO: confirm").count(), 3, "{}", out.bwk);
    assert_eq!(
        out.bwk.matches("// TODO: confirm; from primary_region = \"fra\"").count(),
        2,
        "both residency lines must name the region they came from\n{}",
        out.bwk
    );
}

#[test]
fn fly_import_unknown_region_is_left_unconstrained() {
    let src = "app = \"x\"\nprimary_region = \"gru\"\n[http_service]\ninternal_port = 80\n";
    let out = fly::import(src).unwrap();
    let (file, diags) = parse_and_check(&out.bwk);
    assert!(!any_errors(&diags), "{:?}", diags);
    assert!(file.workloads[0].services[0].residency.is_none());
    assert!(out.notes.iter().any(|n| n.contains("gru")));
}

#[test]
fn compose_import_recovers_stores_dependencies_images_and_sizing() {
    let out = compose::import(COMPOSE).expect("compose import");
    let (file, diags) = parse_and_check(&out.bwk);
    assert!(!any_errors(&diags), "generated BWK must check: {:?}\n{}", diags, out.bwk);
    let w = &file.workloads[0];
    assert_eq!(w.name, "acme-shop");
    let data: Vec<&str> = w.data.iter().map(|d| d.name.as_str()).collect();
    assert!(data.contains(&"db-store"), "{:?}", data);
    assert!(data.contains(&"cache-store"), "{:?}", data);
    let svc = |n: &str| w.services.iter().find(|s| s.name == n).unwrap();
    assert_eq!(svc("db").class, "database");
    assert_eq!(svc("cache").class, "database");
    assert_eq!(svc("web").class, "frontend-static");
    assert_eq!(svc("api").class, "backend-api");
    assert_eq!(svc("worker").class, "background-jobs");
    assert_eq!(svc("api").image.as_deref(), Some("registry.example.com/acme/api:1.4.2"));
    assert_eq!(svc("api").resources.vcpu, Some(0.5), "deploy.resources.reservations.cpus");
    assert_eq!(svc("api").resources.mem_gb, Some(0.5), "512M is binary in compose");
    assert!(svc("api").depends_on.contains(&"db-store".to_string()));
    assert!(svc("api").depends_on.contains(&"cache-store".to_string()));
    assert!(!svc("web").depends_on.iter().any(|d| d == "api-store"));
}

#[test]
fn compose_import_requires_services() {
    assert!(compose::import("version: '3'\n").is_err());
}

#[test]
fn k8s_import_recovers_workloads_volumes_sizing_and_region() {
    let out = k8s::import(K8S).expect("k8s import");
    let (file, diags) = parse_and_check(&out.bwk);
    assert!(!any_errors(&diags), "generated BWK must check: {:?}\n{}", diags, out.bwk);
    let w = &file.workloads[0];
    assert_eq!(w.name, "shop", "workload named after the namespace");
    let data: Vec<&str> = w.data.iter().map(|d| d.name.as_str()).collect();
    assert!(data.contains(&"uploads"), "PVC → data: {:?}", data);
    assert!(data.contains(&"db-store"), "StatefulSet → data: {:?}", data);
    let api = w.services.iter().find(|s| s.name == "api").unwrap();
    assert_eq!(api.class, "backend-api");
    assert_eq!(api.image.as_deref(), Some("registry.example.com/acme/api:1.4.2"));
    assert_eq!(api.residency.as_deref(), Some("eu"), "nodeSelector eu-central-1 → eu");
    assert_eq!(api.resources.vcpu, Some(0.5), "500m");
    assert_eq!(api.resources.mem_gb, Some(0.5), "512Mi");
    assert!(api.depends_on.contains(&"uploads".to_string()));
    let db = w.services.iter().find(|s| s.name == "db").unwrap();
    assert_eq!(db.class, "database");
    assert_eq!(db.resources.mem_gb, Some(2.0), "2Gi");
    assert!(db.depends_on.contains(&"db-store".to_string()));
    let job = w.jobs.iter().find(|j| j.name == "nightly-report").unwrap();
    assert_eq!(job.class, "background-jobs");
    assert_eq!(job.deadline_ms, Some(6.0 * 3_600_000.0), "activeDeadlineSeconds 21600 → 6 h");
    assert_eq!(job.resources.gpu, Some(1.0), "nvidia.com/gpu limit");
    assert!(out.notes.iter().any(|n| n.contains("ignored") && n.contains("ConfigMap")), "{:?}", out.notes);
}

#[test]
fn k8s_import_needs_workload_objects() {
    assert!(k8s::import("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\n").is_err());
}

#[test]
fn terraform_import_recovers_services_stores_residency_and_dependencies() {
    let out = terraform::import(TF).expect("terraform import");
    let (file, diags) = parse_and_check(&out.bwk);
    assert!(!any_errors(&diags), "generated BWK must check: {:?}\n{}", diags, out.bwk);
    let w = &file.workloads[0];
    let data: Vec<&str> = w.data.iter().map(|d| d.name.as_str()).collect();
    assert!(data.contains(&"main-store"), "aws_db_instance → data: {:?}", data);
    assert!(data.contains(&"media"), "aws_s3_bucket → data: {:?}", data);
    let svc = |n: &str| w.services.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("{}\n{}", n, out.bwk));
    assert_eq!(svc("api").class, "backend-api");
    assert_eq!(svc("api").residency.as_deref(), Some("eu"), "provider default eu-central-1");
    assert!(
        svc("api").depends_on.contains(&"main-store".to_string()),
        "reference inside the heredoc should count: {:?}",
        svc("api").depends_on
    );
    assert_eq!(svc("main").class, "database");
    assert!(svc("main").depends_on.contains(&"main-store".to_string()));
    let worker = svc("worker");
    assert!(worker.depends_on.contains(&"main-store".to_string()));
    assert!(worker.depends_on.contains(&"media".to_string()), "explicit depends_on list");
    assert_eq!(svc("reports").residency.as_deref(), Some("eu"), "zone europe-west3-a → eu");
    assert!(out.notes.iter().any(|n| n.contains("aws_security_group")), "{:?}", out.notes);
    // Was `contains(a) || contains(b)` where b is implied by a, so the `||`
    // could never discriminate and the test reduced to the weaker half.
    assert_eq!(out.bwk.matches("instance_type = var.instance_type").count(), 1, "{}", out.bwk);
}

#[test]
fn terraform_import_needs_supported_resources() {
    assert!(terraform::import("resource \"aws_security_group\" \"x\" { name = \"x\" }\n").is_err());
}

// --- helpers --------------------------------------------------------------------

#[test]
fn sanitize_ident_produces_lexable_names() {
    assert_eq!(sanitize_ident("my.app"), "my-app");
    assert_eq!(sanitize_ident("my_app"), "my_app");
    assert_eq!(sanitize_ident("9lives"), "_9lives");
    assert_eq!(sanitize_ident("a--b"), "a-b");
    assert_eq!(sanitize_ident("trailing-"), "trailing");
    assert_eq!(sanitize_ident(""), "app");
    assert_eq!(sanitize_ident("a-_b"), "a-b");
    for s in ["my.app", "9lives", "a--b", "weird name/here", "a-_b"] {
        let id = sanitize_ident(s);
        let src = format!("workload {} {{ }}", id);
        let (_f, diags) = parse_and_check(&src);
        assert!(!any_errors(&diags), "`{}` → `{}` must lex: {:?}", s, id, diags);
    }
}

#[test]
fn classify_image_covers_common_cases() {
    assert_eq!(classify_image("db", Some("postgres:16")), ("database", true));
    assert_eq!(classify_image("search", Some("bitnami/elasticsearch:8")), ("database", true));
    assert_eq!(classify_image("web", Some("nginx:alpine")), ("frontend-static", false));
    assert_eq!(classify_image("worker", Some("registry.example.com/acme/app:1")), ("background-jobs", false));
    assert_eq!(classify_image("api", None), ("backend-api", false));
    assert_eq!(classify_image("api", Some("registry.example.com/acme/web@sha256:abc")), ("backend-api", false));
}

#[test]
fn quantity_parsers_follow_each_ecosystems_conventions() {
    assert_eq!(parse_cpu("500m"), Some(0.5));
    assert_eq!(parse_cpu("2"), Some(2.0));
    assert_eq!(parse_mem_gb("512Mi", true), Some(0.5));
    assert_eq!(parse_mem_gb("1Gi", true), Some(1.0));
    assert_eq!(parse_mem_gb("500M", true), Some(0.4657), "k8s M is decimal");
    assert_eq!(parse_mem_gb("512M", false), Some(0.5), "docker/fly M is binary");
    assert_eq!(parse_mem_gb("1gb", false), Some(1.0));
    assert_eq!(parse_mem_gb("256mb", false), Some(0.25));
    assert_eq!(parse_mem_gb("nonsense", false), None);
}
