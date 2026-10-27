#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;

fn alacoup() -> &'static str {
    env!("CARGO_BIN_EXE_alacoup")
}

fn run(home: &Path, path: &Path, args: &[&str]) -> Output {
    Command::new(alacoup())
        .args(args)
        .env("ALACOUP_HOME", home)
        .env("HOME", home)
        .env("PATH", path)
        .output()
        .expect("AlacoUp should start")
}

fn mock_compiler(bin: &Path, script: &str) {
    fs::create_dir_all(bin).unwrap();
    let compiler = bin.join("alaco");
    fs::write(&compiler, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = fs::metadata(&compiler).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(compiler, permissions).unwrap();
}

#[test]
fn missing_compiler_has_actionable_diagnostic_and_nonzero_status() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let path = temp.path().join("empty-path");
    fs::create_dir(&path).unwrap();

    let output = run(&home, &path, &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Alaco compiler not found"), "{stderr}");
    assert!(stderr.contains("alacoup install"), "{stderr}");
}

#[test]
fn forwards_arguments_verbatim_and_preserves_child_exit_status() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    mock_compiler(
        &bin,
        "printf '%s\\n' \"$@\" > \"$ALACOUP_HOME/args.txt\"\nexit 23",
    );

    let output = run(
        &home,
        &bin,
        &[
            "exec",
            "build",
            "file with spaces.aco",
            "--output",
            "build/output file",
        ],
    );
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        fs::read_to_string(home.join("args.txt")).unwrap(),
        "build\nfile with spaces.aco\n--output\nbuild/output file\n"
    );
}

#[test]
fn doctor_reports_missing_compiler_and_standard_library_without_panicking() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let path = temp.path().join("empty-path");
    fs::create_dir(&path).unwrap();

    let output = run(&home, &path, &["doctor"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Alaco compiler: missing"), "{stdout}");
    assert!(stdout.contains("Standard library:"), "{stdout}");
    assert!(stdout.contains("missing"), "{stdout}");
}

#[test]
fn doctor_succeeds_when_required_tools_and_standard_library_are_present() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let bin = temp.path().join("bin");
    fs::create_dir_all(home.join("manager/.alaco/stdlib/std")).unwrap();
    mock_compiler(&bin, "echo 'alaco 0.1.0'\nexit 0");
    mock_compiler(&bin.join("clang-bin"), "exit 0");
    fs::rename(bin.join("clang-bin/alaco"), bin.join("clang++")).unwrap();

    let output = run(&home.join("manager"), &bin, &["doctor"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Environment looks ready."));
}

#[test]
fn unknown_command_without_arguments_returns_a_useful_error() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let path = temp.path().join("empty-path");
    fs::create_dir(&path).unwrap();

    let output = run(&home, &path, &["unknown-command"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Alaco compiler not found"), "{stderr}");
}

#[test]
fn generates_bash_completions_for_manager_commands() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let path = temp.path().join("empty-path");
    fs::create_dir(&path).unwrap();

    let output = run(&home, &path, &["completions", "bash"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alacoup"), "{stdout}");
    assert!(stdout.contains("install"), "{stdout}");
    assert!(stdout.contains("upgrade"), "{stdout}");
}

#[test]
fn non_executable_compiler_reports_spawn_failure() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("manager");
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("alaco"), "not executable").unwrap();

    let output = run(&home, &bin, &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not start Alaco compiler"),
        "{stderr}"
    );
}

#[test]
fn invalid_version_does_not_modify_existing_home_path() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("file-used-as-home");
    fs::write(&home, "keep this file").unwrap();
    let path = temp.path().join("empty-path");
    fs::create_dir(&path).unwrap();

    let output = run(&home, &path, &["install", "invalid-version"]);
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&home).unwrap(), "keep this file");
}
