//! `rpt --version` reports the version the binary was compiled from.
//!
//! The version has one source — `[workspace.package] version` in the root manifest, inherited by
//! every crate and read at compile time as `CARGO_PKG_VERSION` — so a release tag can be checked
//! against the manifest (`scripts/check-release-version.sh`) rather than against printed text.
#![allow(missing_docs)]

use std::process::Command;

/// The compiled `rpt` binary under test.
const RPT: &str = env!("CARGO_BIN_EXE_rpt");

fn stdout_of(args: &[&str]) -> String {
    let out = Command::new(RPT).args(args).output().expect("run rpt");
    assert!(out.status.success(), "`rpt {args:?}` exited {}", out.status);
    String::from_utf8(out.stdout).expect("utf-8 output")
}

#[test]
fn version_flag_prints_the_compiled_version() {
    let expected = format!("rpt {}", env!("CARGO_PKG_VERSION"));
    for flag in ["-V", "--version"] {
        assert_eq!(stdout_of(&[flag]).trim_end(), expected);
    }
    // A version that is still the placeholder would ship binaries that name no release.
    assert_ne!(env!("CARGO_PKG_VERSION"), "0.0.0");
}

#[test]
fn help_header_carries_the_version() {
    let expected = format!("rpt {}", env!("CARGO_PKG_VERSION"));
    let help = stdout_of(&["--help"]);
    assert!(
        help.starts_with(&expected),
        "{:?}",
        help.lines().next().unwrap_or_default()
    );
}
