//! Bailiwick — reference toolchain.
//!
//! The open, trusted core of the standard: a parser, a unit-aware type
//! checker, policy composition that can only narrow, the normative
//! feasibility evaluator, a canonical IR with JSON and Protobuf codecs,
//! importers from existing deployment files, and a language server. No
//! third-party dependencies, no network access — so anyone can audit it
//! and reproduce its verdicts. Choosing *among* the feasible offers is a job
//! for other tools; this toolchain only decides what is allowed, and why.

pub mod ast;
pub mod builtin;
pub mod cert;
pub mod check;
pub mod diag;
pub mod feasibility;
pub mod fmt;
pub mod import;
pub mod init;
pub mod ir;
pub mod json;
pub mod lexer;
pub mod loader;
pub mod lsp;
pub mod parser;
pub mod plancheck;
pub mod policy;
pub mod proto;
pub mod prove;
pub mod report;
pub mod semantics;

pub use ast::{File, Offer, Policy, Workload};
pub use diag::{any_errors, Diag, Severity};
pub use feasibility::{plan_file, plan_workload, WorkloadPlan};

/// Parse and check a single source string (imports are *not* resolved;
/// use `loader::load` for that). Returns the AST and all diagnostics
/// (parse + semantic). The AST is returned even when diagnostics contain
/// errors, so tools can still show structure.
pub fn parse_and_check(src: &str) -> (File, Vec<Diag>) {
    let parsed = parser::parse(src);
    let mut diags = parsed.diags;
    diags.extend(check::check(&parsed.file));
    (parsed.file, diags)
}

/// Parse a catalog of offers (and anything else) from a source string.
pub fn parse_offers(src: &str) -> (Vec<Offer>, Vec<Diag>) {
    let (file, diags) = parse_and_check(src);
    (file.offers, diags)
}

/// Format a source string canonically. On a parse error the original text
/// is returned unchanged, with the diagnostics, so `fmt` never destroys
/// input it could not understand.
pub fn format_src(src: &str) -> (String, Vec<Diag>) {
    let parsed = parser::parse(src);
    if any_errors(&parsed.diags) {
        return (src.to_string(), parsed.diags);
    }
    (fmt::format_file(&parsed.file), parsed.diags)
}
