//! Smoke tests for the command-line binary.

#![cfg(feature = "cli")]

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_subcommands() {
    Command::cargo_bin("stepq")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("split"))
        .stdout(predicate::str::contains("bom"));
}

#[test]
fn version_matches_cargo() {
    Command::cargo_bin("stepq")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn unimplemented_command_fails_cleanly() {
    Command::cargo_bin("stepq")
        .unwrap()
        .args(["info", "nonexistent.stp"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not implemented"));
}
