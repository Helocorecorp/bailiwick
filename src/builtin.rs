//! What ships inside the toolchain: the compliance rule packs, imported by
//! name (`import "gdpr"`), compiled in so they work offline and in the
//! WebAssembly build, and a given toolchain version always means the same
//! rules. No data about real providers ships: offers come from a catalog
//! you pass, or from the program itself.

/// The rule packs, by import name.
pub const PACKS: &[(&str, &str)] = &[
    ("gdpr", include_str!("../packs/gdpr.bwk")),
    ("eu-sovereign", include_str!("../packs/eu-sovereign.bwk")),
    ("uk-gdpr", include_str!("../packs/uk-gdpr.bwk")),
    ("pipeda", include_str!("../packs/pipeda.bwk")),
    ("canada-residency", include_str!("../packs/canada-residency.bwk")),
    ("quebec-law25", include_str!("../packs/quebec-law25.bwk")),
    ("hipaa", include_str!("../packs/hipaa.bwk")),
    ("ccpa", include_str!("../packs/ccpa.bwk")),
    ("lgpd", include_str!("../packs/lgpd.bwk")),
    ("pipl", include_str!("../packs/pipl.bwk")),
    ("rbi-payments", include_str!("../packs/rbi-payments.bwk")),
    ("my-health-records", include_str!("../packs/my-health-records.bwk")),
];

/// The fictional example offers (`examples/catalog/`): made-up vendors and
/// values, for the playground and the tests. Never a source of real data,
/// and never used unless asked for.
pub const EXAMPLE_CATALOG: &[(&str, &str)] = &[
    ("examples/catalog/core.bwk", include_str!("../examples/catalog/core.bwk")),
    ("examples/catalog/more.bwk", include_str!("../examples/catalog/more.bwk")),
];

/// Whether an import names a built-in pack rather than a file: a bare
/// name, with no path separator and no extension.
pub fn is_pack_name(import: &str) -> bool {
    !import.is_empty() && !import.contains('/') && !import.contains('\\') && !import.contains('.')
}

/// The source of a built-in pack.
pub fn pack(name: &str) -> Option<&'static str> {
    PACKS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// The pack names, for messages.
pub fn pack_names() -> String {
    PACKS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

/// Every fictional example offer.
pub fn example_offers() -> Vec<crate::ast::Offer> {
    EXAMPLE_CATALOG.iter().flat_map(|(_, src)| crate::parser::parse(src).file.offers).collect()
}
