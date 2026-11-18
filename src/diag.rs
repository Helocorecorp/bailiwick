//! Diagnostics: a position plus a message. Errors never panic the tools;
//! they are collected and reported, so `check` and `explain` are total.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub line: usize,
    pub col: usize,
    pub severity: Severity,
    pub msg: String,
}

impl Diag {
    pub fn error(line: usize, col: usize, msg: impl Into<String>) -> Self {
        Diag { line, col, severity: Severity::Error, msg: msg.into() }
    }
    pub fn warning(line: usize, col: usize, msg: impl Into<String>) -> Self {
        Diag { line, col, severity: Severity::Warning, msg: msg.into() }
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}: {}", self.line, self.col, self.severity, self.msg)
    }
}

/// True when any diagnostic is an error.
pub fn any_errors(diags: &[Diag]) -> bool {
    diags.iter().any(|d| d.is_error())
}
