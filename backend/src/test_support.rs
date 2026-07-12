//! Shared fake credentials for tests.
//!
//! Centralizing these gives CodeQL's hardcoded-credential queries (CWE-259/321/798/1204)
//! one definition site to flag instead of one per test file.

/// Fake password used across mock IMAP/SMTP credentials and login test fixtures.
/// Not a real secret - never used outside `#[cfg(test)]` code.
pub(crate) const FAKE_PASSWORD: &str = "hunter2";
