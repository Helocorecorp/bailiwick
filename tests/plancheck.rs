//! `check-plan`: real infrastructure plans held to the rules.

use bailiwick::plancheck::{self, locate};

fn fixture(f: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(f)).unwrap()
}

fn program(src: &str) -> bailiwick::File {
    let loaded = bailiwick::loader::load_source(std::path::Path::new("t.bwk"), src, Vec::new());
    assert!(!bailiwick::any_errors(&loaded.diags), "{:?}", loaded.diags);
    loaded.file
}

fn verdicts(src: &str, resources: &[plancheck::PlannedResource]) -> Vec<(String, String)> {
    plancheck::check(&program(src), resources)
        .into_iter()
        .map(|c| {
            let v = if let Some(s) = c.skipped {
                format!("skip: {}", s)
            } else if c.violations.is_empty() {
                "ok".into()
            } else {
                c.violations.iter().map(|e| e.kind()).collect::<Vec<_>>().join(",")
            };
            (c.resource.address, v)
        })
        .collect()
}

#[test]
fn regions_resolve_to_countries_and_subdivisions() {
    assert_eq!(locate("aws", "eu-central-1"), Some(("de", Some("he"))));
    assert_eq!(locate("aws", "eu-central-1b"), Some(("de", Some("he"))));
    assert_eq!(locate("gcp", "europe-west3-a"), Some(("de", Some("he"))));
    assert_eq!(locate("gcp", "northamerica-northeast1"), Some(("ca", Some("qc"))));
    assert_eq!(locate("azure", "West Europe"), Some(("nl", None)));
    assert_eq!(locate("hetzner", "fsn1-dc14"), Some(("de", Some("sn"))));
    assert_eq!(locate("digitalocean", "tor1"), Some(("ca", Some("on"))));
    assert_eq!(locate("gcp", "EU"), None, "multi-regions are not a single place");
}

#[test]
fn terraform_resources_take_their_provider_s_region() {
    let rs = plancheck::terraform_resources(&fixture("terraform-plan.json")).unwrap();
    let got: Vec<(String, Option<String>)> = rs.iter().map(|r| (r.address.clone(), r.region.clone())).collect();
    assert_eq!(
        got,
        vec![
            ("aws_instance.api".into(), Some("eu-central-1".into())),
            ("aws_db_instance.main".into(), Some("eu-central-1".into())),
            ("aws_s3_bucket.backups".into(), Some("us-east-1".into())),
            ("google_compute_instance.reports[0]".into(), Some("europe-west3-a".into())),
            ("module.cache.hcloud_server.redis".into(), Some("fsn1".into())),
        ],
        "IAM roles and data sources are left out; aliases and modules resolve"
    );
}

const SHOP: &str = "workload shop {
  data customers { sensitivity = S3  residency = eu }
  service api : backend-api { depends_on = customers }
  job reports : background-jobs { deadline = 6 h }
}
";

#[test]
fn a_resident_plan_passes_until_a_bucket_leaves() {
    let rs = plancheck::terraform_resources(&fixture("terraform-plan.json")).unwrap();
    let v = verdicts(SHOP, &rs);
    assert_eq!(v[0].1, "ok");
    assert_eq!(v[2], ("aws_s3_bucket.backups".into(), "residency".into()), "{:?}", v);
    assert_eq!(v[3].1, "ok", "the job has no residency of its own");
    let with_gdpr = verdicts(&format!("import \"gdpr\"\n{}", SHOP), &rs);
    assert_eq!(with_gdpr[2].1, "residency", "US is not GDPR-adequate by location");
}

#[test]
fn sovereignty_follows_the_operator_not_the_server() {
    let rs = plancheck::terraform_resources(&fixture("terraform-plan.json")).unwrap();
    let v = verdicts(&format!("import \"eu-sovereign\"\n{}", SHOP), &rs);
    assert_eq!(v[0].1, "jurisdiction", "AWS Frankfurt is within reach of US law");
    assert_eq!(v[4].1, "ok", "Hetzner Falkenstein is not");
    // With customer-held keys, AWS Frankfurt is fine.
    let mut keyed = rs.clone();
    keyed[0].customer_keys = true;
    assert_eq!(verdicts(&format!("import \"eu-sovereign\"\n{}", SHOP), &keyed)[0].1, "ok");
}

#[test]
fn kubernetes_workloads_are_checked_per_pinned_region() {
    let rs = plancheck::kubernetes_resources(&fixture("k8s-placed.yaml"), "aws").unwrap();
    let v = verdicts(&format!("import \"eu-sovereign\"\n{}", SHOP), &rs);
    assert_eq!(
        v,
        vec![
            ("Deployment/api (eu-central-1)".into(), "jurisdiction".into()),
            ("StatefulSet/customers (eu-central-1)".into(), "ok".into()),
            ("StatefulSet/customers (us-east-1)".into(), "residency".into()),
            (
                "CronJob/reports".into(),
                "skip: not pinned to a region (no nodeSelector or node affinity on topology.kubernetes.io/region)".into()
            ),
        ]
    );
}
