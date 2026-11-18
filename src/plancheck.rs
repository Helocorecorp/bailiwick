//! `bailiwick check-plan`: hold real infrastructure to the rules.
//!
//! Terraform (`terraform show -json plan.out`) and Kubernetes manifests
//! say where things will actually be created. This reads each resource's
//! provider and region, turns it into a location (country, subdivision,
//! the jurisdictions that reach its operator), and checks it against what
//! the program allows: the residency of the service, job or data it
//! belongs to, and every policy and rule pack in force.
//!
//! Global resources (identity, DNS, firewall rules, networks) hold no data
//! and run nothing, so they are left out; resources from providers this
//! does not know are left out too.
//!
//! Only what a plan reveals is checked — residency, denied countries,
//! vendors and jurisdictions, and key custody. Trust, capacity and
//! features are properties of offers, not of a plan, so `explain` covers
//! them. A resource whose region the plan does not state is listed as
//! skipped, never passed silently.
//!
//! A resource belongs to a unit by its `bailiwick` tag or label (on
//! Kubernetes, the `bailiwick` label), else by having the unit's name.
//! A `bailiwick-keys = customer` tag says the customer holds its keys.

use crate::ast::{customer_keys, File, Offer, Resources};
use crate::feasibility::{check_location, plan_workload, Exclusion};
use crate::import::yaml_min::{self, Node};
use crate::json::{self, Json};
use crate::policy::effective;
use crate::semantics::{fold_residencies, fold_residency, Residency};

/// Where a cloud region is: country and, where it is known, subdivision.
/// Vendors are named as in the catalog.
pub fn locate(vendor: &str, region: &str) -> Option<(&'static str, Option<&'static str>)> {
    let r = region.trim().to_lowercase().replace(' ', "");
    let table: &[(&str, &str, Option<&str>)] = match vendor {
        "aws" => &[
            ("us-east-1", "us", Some("va")), ("us-east-2", "us", Some("oh")), ("us-west-1", "us", Some("ca")),
            ("us-west-2", "us", Some("or")), ("us-gov-west-1", "us", None), ("us-gov-east-1", "us", None),
            ("ca-central-1", "ca", Some("qc")), ("ca-west-1", "ca", Some("ab")), ("mx-central-1", "mx", None),
            ("sa-east-1", "br", Some("sp")), ("eu-central-1", "de", Some("he")), ("eu-central-2", "ch", None),
            ("eu-west-1", "ie", None), ("eu-west-2", "gb", None), ("eu-west-3", "fr", None),
            ("eu-north-1", "se", None), ("eu-south-1", "it", None), ("eu-south-2", "es", None),
            ("il-central-1", "il", None), ("me-south-1", "bh", None), ("me-central-1", "ae", None),
            ("af-south-1", "za", None), ("ap-east-1", "hk", None), ("ap-south-1", "in", Some("mh")),
            ("ap-south-2", "in", Some("tg")), ("ap-southeast-1", "sg", None), ("ap-southeast-2", "au", Some("nsw")),
            ("ap-southeast-3", "id", None), ("ap-southeast-4", "au", Some("vic")), ("ap-southeast-5", "my", None),
            ("ap-southeast-7", "th", None), ("ap-northeast-1", "jp", None), ("ap-northeast-2", "kr", None),
            ("ap-northeast-3", "jp", None), ("cn-north-1", "cn", None), ("cn-northwest-1", "cn", None),
        ],
        "gcp" => &[
            ("us-central1", "us", Some("ia")), ("us-east1", "us", Some("sc")), ("us-east4", "us", Some("va")),
            ("us-east5", "us", Some("oh")), ("us-south1", "us", Some("tx")), ("us-west1", "us", Some("or")),
            ("us-west2", "us", Some("ca")), ("us-west3", "us", Some("ut")), ("us-west4", "us", Some("nv")),
            ("northamerica-northeast1", "ca", Some("qc")), ("northamerica-northeast2", "ca", Some("on")),
            ("northamerica-south1", "mx", None), ("southamerica-east1", "br", Some("sp")),
            ("southamerica-west1", "cl", None), ("europe-west1", "be", None), ("europe-west2", "gb", None),
            ("europe-west3", "de", Some("he")), ("europe-west4", "nl", None), ("europe-west6", "ch", None),
            ("europe-west8", "it", None), ("europe-west9", "fr", None), ("europe-west10", "de", Some("be")),
            ("europe-west12", "it", None), ("europe-central2", "pl", None), ("europe-north1", "fi", None),
            ("europe-north2", "se", None), ("europe-southwest1", "es", None), ("me-west1", "il", None),
            ("me-central1", "qa", None), ("me-central2", "sa", None), ("africa-south1", "za", None),
            ("asia-east1", "tw", None), ("asia-east2", "hk", None), ("asia-northeast1", "jp", None),
            ("asia-northeast2", "jp", None), ("asia-northeast3", "kr", None), ("asia-south1", "in", Some("mh")),
            ("asia-south2", "in", Some("dl")), ("asia-southeast1", "sg", None), ("asia-southeast2", "id", None),
            ("australia-southeast1", "au", Some("nsw")), ("australia-southeast2", "au", Some("vic")),
        ],
        "azure" => &[
            ("eastus", "us", Some("va")), ("eastus2", "us", Some("va")), ("centralus", "us", Some("ia")),
            ("northcentralus", "us", Some("il")), ("southcentralus", "us", Some("tx")), ("westus", "us", Some("ca")),
            ("westus2", "us", Some("wa")), ("westus3", "us", Some("az")), ("westcentralus", "us", Some("wy")),
            ("canadacentral", "ca", Some("on")), ("canadaeast", "ca", Some("qc")), ("brazilsouth", "br", Some("sp")),
            ("mexicocentral", "mx", None), ("northeurope", "ie", None), ("westeurope", "nl", None),
            ("francecentral", "fr", None), ("francesouth", "fr", None), ("germanywestcentral", "de", Some("he")),
            ("germanynorth", "de", None), ("uksouth", "gb", None), ("ukwest", "gb", None),
            ("swedencentral", "se", None), ("norwayeast", "no", None), ("norwaywest", "no", None),
            ("switzerlandnorth", "ch", None), ("switzerlandwest", "ch", None), ("polandcentral", "pl", None),
            ("italynorth", "it", None), ("spaincentral", "es", None), ("japaneast", "jp", None),
            ("japanwest", "jp", None), ("koreacentral", "kr", None), ("koreasouth", "kr", None),
            ("centralindia", "in", None), ("southindia", "in", None), ("westindia", "in", None),
            ("southeastasia", "sg", None), ("eastasia", "hk", None), ("australiaeast", "au", Some("nsw")),
            ("australiasoutheast", "au", Some("vic")), ("australiacentral", "au", Some("act")),
            ("uaenorth", "ae", None), ("qatarcentral", "qa", None), ("southafricanorth", "za", None),
            ("israelcentral", "il", None),
        ],
        "digitalocean" => &[
            ("nyc1", "us", Some("ny")), ("nyc2", "us", Some("ny")), ("nyc3", "us", Some("ny")),
            ("sfo2", "us", Some("ca")), ("sfo3", "us", Some("ca")), ("atl1", "us", Some("ga")),
            ("ams3", "nl", None), ("fra1", "de", Some("he")), ("lon1", "gb", None), ("tor1", "ca", Some("on")),
            ("sgp1", "sg", None), ("blr1", "in", Some("ka")), ("syd1", "au", Some("nsw")),
        ],
        "hetzner" => &[
            ("fsn1", "de", Some("sn")), ("nbg1", "de", Some("by")), ("hel1", "fi", None),
            ("ash", "us", Some("va")), ("hil", "us", Some("or")), ("sin", "sg", None),
        ],
        _ => &[],
    };
    // Zones and datacenters reduce to their region: `eu-central-1a`,
    // `europe-west3-b`, `fsn1-dc14`.
    let mut cands = vec![r.clone()];
    if let Some(i) = r.rfind('-') {
        cands.push(r[..i].to_string());
    }
    if vendor == "aws" && r.ends_with(|c: char| c.is_ascii_lowercase()) {
        cands.push(r[..r.len() - 1].to_string());
    }
    for c in &cands {
        if let Some((_, country, sub)) = table.iter().find(|(name, _, _)| name == c) {
            return Some((country, *sub));
        }
    }
    None
}

/// The jurisdiction whose law reaches a vendor wherever it operates: its
/// headquarters. Matches what the catalog declares.
pub fn home_jurisdiction(vendor: &str) -> Option<&'static str> {
    match vendor {
        "aws" | "gcp" | "azure" | "digitalocean" | "linode" | "vultr" | "fly" | "cloudflare" => Some("us"),
        "hetzner" => Some("de"),
        "ovh" | "scaleway" => Some("fr"),
        _ => None,
    }
}

/// One resource from a plan.
#[derive(Clone, Debug)]
pub struct PlannedResource {
    pub address: String,
    pub name: String,
    pub vendor: Option<String>,
    pub region: Option<String>,
    /// The `bailiwick` tag or label: the unit or data it belongs to.
    pub unit_tag: Option<String>,
    pub customer_keys: bool,
    /// Why the resource has no location to check, when it has none.
    pub not_located: Option<String>,
}

/// Resource types that are global, or hold no data and run nothing:
/// identity, DNS, CDN configuration, networking rules.
fn is_global(ty: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "aws_iam_", "aws_route53", "aws_cloudfront", "aws_organizations", "aws_security_group",
        "aws_vpc", "aws_subnet", "aws_route", "aws_internet_gateway", "aws_acm_",
        "google_project", "google_service_account", "google_compute_firewall", "google_compute_network",
        "google_dns_", "azurerm_role", "azurerm_resource_group", "azurerm_dns_", "azurerm_network_security",
        "azurerm_virtual_network", "azurerm_subnet", "digitalocean_domain", "digitalocean_record",
        "digitalocean_firewall", "digitalocean_project", "digitalocean_vpc", "hcloud_firewall", "hcloud_network",
        "hcloud_ssh_key", "digitalocean_ssh_key", "aws_key_pair",
    ];
    PREFIXES.iter().any(|p| ty.starts_with(p))
}
#[derive(Clone, Debug)]
pub struct ResourceCheck {
    pub resource: PlannedResource,
    /// `country` or `country-sub`, when located.
    pub location: Option<String>,
    /// What the resource was held to: "service `api`", "data `db`", or
    /// "the policies in force".
    pub held_to: String,
    pub violations: Vec<Exclusion>,
    /// Why it was not checked, when it was not.
    pub skipped: Option<String>,
}

fn tf_vendor(provider_or_type: &str) -> Option<&'static str> {
    let p = provider_or_type.rsplit('/').next().unwrap_or(provider_or_type);
    let p = p.split('_').next().unwrap_or(p);
    match p {
        "aws" => Some("aws"),
        "google" | "google-beta" => Some("gcp"),
        "azurerm" => Some("azure"),
        "digitalocean" => Some("digitalocean"),
        "hcloud" => Some("hetzner"),
        _ => None,
    }
}

fn str_at<'a>(j: &'a Json, path: &[&str]) -> Option<&'a str> {
    let mut cur = j;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str()
}

/// Tags and labels: a map, or (DigitalOcean) a list of `key:value`.
fn tag(values: &Json, key: &str) -> Option<String> {
    for field in ["tags", "labels", "tags_all"] {
        match values.get(field) {
            Some(Json::Obj(kv)) => {
                if let Some((_, v)) = kv.iter().find(|(k, _)| k == key) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
            Some(Json::Arr(items)) => {
                for i in items {
                    if let Some((k, v)) = i.as_str().and_then(|s| s.split_once(':')) {
                        if k == key {
                            return Some(v.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// `aws_instance.web[0]` → `aws_instance.web`; module prefixes are kept.
fn strip_index(addr: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in addr.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Provider configuration regions and which provider configuration each
/// resource uses, from the plan's `configuration` section.
/// Pairs of names: (provider configuration, region) or (resource, provider configuration).
type Pairs = Vec<(String, String)>;

fn tf_provider_regions(plan: &Json) -> (Pairs, Pairs) {
    let mut regions = Vec::new();
    if let Some(Json::Obj(pcs)) = plan.get("configuration").and_then(|c| c.get("provider_config")) {
        for (key, pc) in pcs {
            for attr in ["region", "location"] {
                if let Some(r) = str_at(pc, &["expressions", attr, "constant_value"]) {
                    regions.push((key.clone(), r.to_string()));
                    break;
                }
            }
        }
    }
    let mut uses = Vec::new();
    fn walk(m: &Json, prefix: &str, uses: &mut Vec<(String, String)>) {
        for r in m.get("resources").and_then(|r| r.as_arr()).unwrap_or(&[]) {
            if let (Some(a), Some(k)) = (r.get("address").and_then(|x| x.as_str()), r.get("provider_config_key").and_then(|x| x.as_str())) {
                uses.push((format!("{}{}", prefix, a), k.to_string()));
            }
        }
        if let Some(Json::Obj(calls)) = m.get("module_calls") {
            for (name, call) in calls {
                if let Some(sub) = call.get("module") {
                    walk(sub, &format!("{}module.{}.", prefix, name), uses);
                }
            }
        }
    }
    if let Some(root) = plan.get("configuration").and_then(|c| c.get("root_module")) {
        walk(root, "", &mut uses);
    }
    (regions, uses)
}

/// Read the resources of a Terraform plan (`terraform show -json`).
pub fn terraform_resources(plan_json: &str) -> Result<Vec<PlannedResource>, String> {
    let plan = json::parse(plan_json).map_err(|e| format!("not JSON: {}", e))?;
    let root = plan
        .get("planned_values")
        .and_then(|p| p.get("root_module"))
        .ok_or("not a Terraform plan: no `planned_values.root_module` (use `terraform show -json plan.out`)")?;
    let (regions, uses) = tf_provider_regions(&plan);
    let mut out = Vec::new();
    fn walk(m: &Json, out: &mut Vec<Json>) {
        for r in m.get("resources").and_then(|r| r.as_arr()).unwrap_or(&[]) {
            out.push(r.clone());
        }
        for c in m.get("child_modules").and_then(|c| c.as_arr()).unwrap_or(&[]) {
            walk(c, out);
        }
    }
    let mut all = Vec::new();
    walk(root, &mut all);
    for r in &all {
        if r.get("mode").and_then(|m| m.as_str()) == Some("data") {
            continue;
        }
        let address = r.get("address").and_then(|a| a.as_str()).unwrap_or("").to_string();
        let ty = r.get("type").and_then(|a| a.as_str()).unwrap_or("");
        let name = r.get("name").and_then(|a| a.as_str()).unwrap_or("").to_string();
        let provider = r.get("provider_name").and_then(|a| a.as_str()).unwrap_or(ty);
        let vendor = tf_vendor(provider).or_else(|| tf_vendor(ty));
        let values = r.get("values").cloned().unwrap_or(Json::Null);
        let own = ["region", "location", "zone", "availability_zone", "datacenter"]
            .iter()
            .find_map(|k| values.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from));
        if is_global(ty) || vendor.is_none() {
            continue;
        }
        let region = own.or_else(|| {
            let key = uses
                .iter()
                .find(|(a, _)| *a == strip_index(&address))
                .map(|(_, k)| k.clone())
                .or_else(|| provider.rsplit('/').next().map(String::from))?;
            regions.iter().find(|(k, _)| *k == key).map(|(_, r)| r.clone())
        });
        out.push(PlannedResource {
            address,
            name,
            vendor: vendor.map(String::from),
            region,
            unit_tag: tag(&values, "bailiwick"),
            customer_keys: tag(&values, "bailiwick-keys").as_deref() == Some("customer"),
            not_located: None,
        });
    }
    Ok(out)
}

fn node_str<'a>(n: &'a Node, path: &[&str]) -> Option<&'a str> {
    let mut cur = n;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str()
}

/// The regions a pod spec pins itself to, by `nodeSelector` or required
/// node affinity on `topology.kubernetes.io/region`.
fn pod_regions(pod: &Node) -> Vec<String> {
    const KEYS: [&str; 2] = ["topology.kubernetes.io/region", "failure-domain.beta.kubernetes.io/region"];
    let mut out = Vec::new();
    for k in KEYS {
        if let Some(r) = pod.get("nodeSelector").and_then(|n| n.get(k)).and_then(|v| v.as_str()) {
            out.push(r.to_string());
        }
    }
    let terms = pod
        .get("affinity")
        .and_then(|a| a.get("nodeAffinity"))
        .and_then(|a| a.get("requiredDuringSchedulingIgnoredDuringExecution"))
        .and_then(|a| a.get("nodeSelectorTerms"))
        .and_then(|t| t.as_seq())
        .unwrap_or(&[]);
    for t in terms {
        for e in t.get("matchExpressions").and_then(|m| m.as_seq()).unwrap_or(&[]) {
            let key = e.get("key").and_then(|k| k.as_str()).unwrap_or("");
            if KEYS.contains(&key) && e.get("operator").and_then(|o| o.as_str()) == Some("In") {
                for v in e.get("values").and_then(|v| v.as_seq()).unwrap_or(&[]) {
                    if let Some(s) = v.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Read the workloads of Kubernetes manifests running on a cluster at
/// `vendor`. A workload pinned to several regions yields one resource per
/// region, each checked.
pub fn kubernetes_resources(manifests: &str, vendor: &str) -> Result<Vec<PlannedResource>, String> {
    let docs = yaml_min::parse_all(manifests)?;
    let mut out = Vec::new();
    for d in &docs {
        let kind = node_str(d, &["kind"]).unwrap_or("");
        let pod = match kind {
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job" => d.get("spec").and_then(|s| s.get("template")).and_then(|t| t.get("spec")),
            "CronJob" => d
                .get("spec")
                .and_then(|s| s.get("jobTemplate"))
                .and_then(|s| s.get("spec"))
                .and_then(|s| s.get("template"))
                .and_then(|t| t.get("spec")),
            "Pod" => d.get("spec"),
            _ => continue,
        };
        let name = node_str(d, &["metadata", "name"]).unwrap_or("").to_string();
        let unit_tag = node_str(d, &["metadata", "labels", "bailiwick"]).map(String::from);
        let customer_keys = node_str(d, &["metadata", "labels", "bailiwick-keys"]) == Some("customer");
        let regions = pod.map(pod_regions).unwrap_or_default();
        let address = format!("{}/{}", kind, name);
        if regions.is_empty() {
            let not_located = Some("not pinned to a region (no nodeSelector or node affinity on topology.kubernetes.io/region)".into());
            out.push(PlannedResource { address, name, vendor: Some(vendor.into()), region: None, unit_tag, customer_keys, not_located });
        } else {
            for r in regions {
                out.push(PlannedResource {
                    address: format!("{} ({})", address, r),
                    name: name.clone(),
                    vendor: Some(vendor.into()),
                    region: Some(r),
                    unit_tag: unit_tag.clone(),
                    customer_keys,
                    not_located: None,
                });
            }
        }
    }
    Ok(out)
}

/// What a resource is held to: a residency and a custody requirement.
struct Target {
    held_to: String,
    residency: Residency,
    customer_keys: Option<String>,
}

fn target_for(file: &File, r: &PlannedResource) -> Target {
    let eff = effective(&file.policies);
    let key = r.unit_tag.clone().unwrap_or_else(|| r.name.clone());
    let alt = format!("{}-store", r.name);
    for w in &file.workloads {
        let plan = plan_workload(w, &[], &file.policies);
        if let Some(u) = plan.units.iter().find(|u| u.name == key) {
            return Target {
                held_to: format!("{} `{}`", u.kind, u.name),
                residency: u.residency.clone(),
                customer_keys: u.customer_keys.clone(),
            };
        }
        if let Some(d) = w.data.iter().find(|d| d.name == key || (r.unit_tag.is_none() && d.name == alt)) {
            let residency =
                fold_residencies(fold_residency(None, d.residency.as_deref()), eff.residency.as_ref()).unwrap_or(Residency::Global);
            let keys = if customer_keys(&d.keys) {
                Some(format!("data `{}`", d.name))
            } else {
                eff.customer_keys.as_ref().map(|p| format!("policy `{}`", p))
            };
            return Target { held_to: format!("data `{}`", d.name), residency, customer_keys: keys };
        }
    }
    Target {
        held_to: "the policies in force".into(),
        residency: eff.residency.clone().unwrap_or(Residency::Global),
        customer_keys: eff.customer_keys.as_ref().map(|p| format!("policy `{}`", p)),
    }
}

/// Check every resource against the program.
pub fn check(file: &File, resources: &[PlannedResource]) -> Vec<ResourceCheck> {
    let eff = effective(&file.policies);
    resources
        .iter()
        .map(|r| {
            let t = target_for(file, r);
            let mut out = ResourceCheck {
                resource: r.clone(),
                location: None,
                held_to: t.held_to.clone(),
                violations: Vec::new(),
                skipped: None,
            };
            let Some(vendor) = r.vendor.as_deref() else {
                out.skipped = Some("provider not recognised".into());
                return out;
            };
            let Some(region) = r.region.as_deref() else {
                out.skipped = Some(r.not_located.clone().unwrap_or_else(|| "the plan does not say where".into()));
                return out;
            };
            let Some((country, sub)) = locate(vendor, region) else {
                out.skipped = Some(format!("location `{}` is not a single region this checker knows", region));
                return out;
            };
            out.location = Some(match sub {
                Some(s) => format!("{}-{}", country, s),
                None => country.to_string(),
            });
            let offer = Offer {
                id: r.address.clone(),
                vendor: Some(vendor.to_string()),
                country: Some(country.to_string()),
                subdivision: sub.map(String::from),
                trust: None,
                classes: Vec::new(),
                price: None,
                resources: Resources::default(),
                features: Vec::new(),
                jurisdictions: home_jurisdiction(vendor).filter(|j| *j != country).map(|j| vec![j.to_string()]).unwrap_or_default(),
                keys: if r.customer_keys { Some("customer".into()) } else { None },
                // A plan says where, not how fast: latency is checked by `explain`.
                latency: Vec::new(),
                line: 0,
                col: 0,
            };
            out.violations = check_location(&offer, &t.residency, t.customer_keys.as_deref(), &eff);
            out
        })
        .collect()
}

/// A plain-text report; the second value is whether everything passed.
pub fn report(checks: &[ResourceCheck]) -> (String, bool) {
    let checked = checks.iter().filter(|c| c.skipped.is_none()).count();
    let failed = checks.iter().filter(|c| !c.violations.is_empty()).count();
    let skipped = checks.len() - checked;
    let mut s = format!(
        "check-plan: {} resource(s) checked, {} violate the rules, {} skipped\n",
        checked, failed, skipped
    );
    for c in checks {
        let r = &c.resource;
        let mark = if c.skipped.is_some() {
            "skip"
        } else if c.violations.is_empty() {
            "ok  "
        } else {
            "FAIL"
        };
        let loc = match (&c.location, &r.vendor) {
            (Some(l), Some(v)) => format!("{} {}", v, l),
            _ => r.region.clone().unwrap_or_else(|| "-".into()),
        };
        s.push_str(&format!("  {} {}  [{}]  held to {}\n", mark, r.address, loc, c.held_to));
        if let Some(why) = &c.skipped {
            s.push_str(&format!("       not checked: {}\n", why));
        }
        for v in &c.violations {
            s.push_str(&format!("       {}\n", v.reason()));
        }
    }
    (s, failed == 0)
}

/// A Markdown version of `report`, for pull-request comments.
pub fn markdown(checks: &[ResourceCheck]) -> String {
    let failed = checks.iter().filter(|c| !c.violations.is_empty()).count();
    let checked = checks.iter().filter(|c| c.skipped.is_none()).count();
    let mut s = format!(
        "#### Infrastructure plan: {}\n\n{} resource(s) checked, {} violate the rules, {} skipped.\n\n",
        if failed == 0 { "follows the rules" } else { "**breaks the rules**" },
        checked,
        failed,
        checks.len() - checked
    );
    s.push_str("| Resource | Where | Held to | Verdict |\n|---|---|---|---|\n");
    for c in checks {
        let r = &c.resource;
        let loc = match (&c.location, &r.vendor) {
            (Some(l), Some(v)) => format!("{} {}", v, l),
            _ => r.region.clone().unwrap_or_else(|| "?".into()),
        };
        let verdict = if let Some(why) = &c.skipped {
            format!("not checked: {}", why)
        } else if c.violations.is_empty() {
            "ok".into()
        } else {
            format!("**{}**", c.violations.iter().map(|v| v.reason()).collect::<Vec<_>>().join("; "))
        };
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            r.address,
            loc,
            c.held_to,
            verdict.replace('|', "\\|")
        ));
    }
    s
}
