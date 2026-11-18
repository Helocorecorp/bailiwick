//! Robustness: every parser and importer must return a result — never
//! panic — on arbitrary input. A small deterministic PRNG mutates the
//! sample files (deletions, insertions of bracket/quote soup, truncation)
//! and feeds the results through everything that reads text.

use std::panic;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const SOUP: &[&str] = &[
    "{", "}", "[", "]", "(", ")", "\"", "'", "#", "//", "/*", "*/", "$", "=", "<=", ">=", ":", ",",
    ".", "-", "\n", "  ", "\t", "---", "- ", "|", ">", "&x", "*x", "${", "<<EOF", "é", "😀", "\u{0}",
    "0", "9999999999999999999999", "1e400", "S3", "T9", "eu", "mem", "GB", "ms", "never", "for",
    "import", "policy", "workload", "offer", "resource", "kind:", "services:",
];

fn mutate(rng: &mut Rng, base: &str) -> String {
    let mut chars: Vec<char> = base.chars().collect();
    let ops = 1 + rng.below(6);
    for _ in 0..ops {
        match rng.below(4) {
            0 => {
                // delete a range
                if !chars.is_empty() {
                    let a = rng.below(chars.len());
                    let b = (a + rng.below(40)).min(chars.len());
                    chars.drain(a..b);
                }
            }
            1 => {
                // insert soup
                let at = rng.below(chars.len() + 1);
                let s: Vec<char> = rng.pick(SOUP).chars().collect();
                chars.splice(at..at, s);
            }
            2 => {
                // truncate
                let at = rng.below(chars.len() + 1);
                chars.truncate(at);
            }
            _ => {
                // duplicate a range
                if !chars.is_empty() {
                    let a = rng.below(chars.len());
                    let b = (a + rng.below(60)).min(chars.len());
                    let dup: Vec<char> = chars[a..b].to_vec();
                    let at = rng.below(chars.len() + 1);
                    chars.splice(at..at, dup);
                }
            }
        }
    }
    chars.into_iter().collect()
}

fn no_panic<F: FnOnce() + panic::UnwindSafe>(what: &str, input: &str, f: F) {
    if panic::catch_unwind(f).is_err() {
        panic!("{} panicked on input:\n{}", what, input);
    }
}

#[test]
fn awl_front_end_never_panics() {
    let seeds = [
        include_str!("../examples/storefront.bwk"),
        include_str!("../examples/storefront-eu-policy.bwk"),
        include_str!("../examples/edge-site.bwk"),
        include_str!("../examples/catalog/core.bwk"),
        include_str!("../examples/policies/eu-only.bwk"),
    ];
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for i in 0..1500 {
        let base = seeds[i % seeds.len()];
        let input = mutate(&mut rng, base);
        no_panic("parse_and_check", &input, || {
            let (file, _diags) = bailiwick::parse_and_check(&input);
            let _ = bailiwick::ir::to_string(&file);
            let _ = bailiwick::fmt::format_file(&file);
            let bytes = bailiwick::proto::encode_file(&file);
            let _ = bailiwick::proto::decode_file(&bytes);
            let (offers, _) = bailiwick::parse_offers(seeds[3]);
            let _ = bailiwick::plan_file(&file, &offers);
            let _ = bailiwick::report::json_string(&bailiwick::plan_file(&file, &file.offers));
        });
        no_panic("lsp", &input, || {
            let _ = bailiwick::lsp::diagnostics("untitled:x", &input);
            let _ = bailiwick::lsp::completions(&input, 3, 10);
            let _ = bailiwick::lsp::hover(&input, 2, 4);
            let _ = bailiwick::lsp::symbols(&input);
            let _ = bailiwick::lsp::format_edits(&input);
            let _ = bailiwick::lsp::definition("untitled:x", &input, 19, 20);
        });
    }
}

#[test]
fn readers_and_importers_never_panic() {
    let toml = include_str!("../examples/import/fly.toml");
    let yaml = include_str!("../examples/import/docker-compose.yml");
    let k8s = include_str!("../examples/import/k8s.yaml");
    let hcl = include_str!("../examples/import/main.tf");
    let mut rng = Rng(0xD1B54A32D192ED03);
    for i in 0..1200 {
        match i % 4 {
            0 => {
                let input = mutate(&mut rng, toml);
                no_panic("toml/fly", &input, || {
                    let _ = bailiwick::import::toml_min::parse(&input);
                    let _ = bailiwick::import::fly::import(&input);
                });
            }
            1 => {
                let input = mutate(&mut rng, yaml);
                no_panic("yaml/compose", &input, || {
                    let _ = bailiwick::import::yaml_min::parse(&input);
                    let _ = bailiwick::import::yaml_min::parse_all(&input);
                    let _ = bailiwick::import::compose::import(&input);
                });
            }
            2 => {
                let input = mutate(&mut rng, k8s);
                no_panic("yaml/k8s", &input, || {
                    let _ = bailiwick::import::k8s::import(&input);
                });
            }
            _ => {
                let input = mutate(&mut rng, hcl);
                no_panic("hcl/terraform", &input, || {
                    let _ = bailiwick::import::hcl_min::parse(&input);
                    let _ = bailiwick::import::terraform::import(&input);
                });
            }
        }
    }
}

#[test]
fn json_parser_never_panics() {
    let (file, _) = bailiwick::parse_and_check(include_str!("../examples/storefront.bwk"));
    let base = bailiwick::ir::to_string(&file);
    let mut rng = Rng(0xA0761D6478BD642F);
    for _ in 0..800 {
        let input = mutate(&mut rng, &base);
        no_panic("json", &input, || {
            let _ = bailiwick::json::parse(&input);
        });
    }
}

#[test]
fn deep_nesting_is_rejected_not_overflowed() {
    let deep_json = "[".repeat(5000);
    assert!(bailiwick::json::parse(&deep_json).is_err());
    let deep_toml = format!("a = {}", "[".repeat(5000));
    assert!(bailiwick::import::toml_min::parse(&deep_toml).is_err());
    let deep_hcl = format!("a = {}", "[".repeat(5000));
    assert!(bailiwick::import::hcl_min::parse(&deep_hcl).is_err());
    let mut deep_yaml = String::new();
    for i in 0..400 {
        deep_yaml.push_str(&" ".repeat(i));
        deep_yaml.push_str("k:\n");
    }
    assert!(bailiwick::import::yaml_min::parse(&deep_yaml).is_err());
}
