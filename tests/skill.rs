// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::fs;
use std::process::Command;

#[test]
fn print_and_path_use_the_embedded_skill() {
    let home = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_mxl");
    let printed = Command::new(cli)
        .args(["agent", "skill", "print"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.path())
        .output()
        .unwrap();
    assert!(printed.status.success());
    assert_eq!(printed.stdout, mixlayer::skill::TEXT.as_bytes());
    let output = Command::new(cli)
        .args(["agent", "skill", "path"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let path = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(path.starts_with(home.path()));
    assert_eq!(
        fs::read_to_string(path.join("SKILL.md")).unwrap(),
        mixlayer::skill::TEXT
    );
    assert_eq!(
        fs::read_to_string(path.join("references/privacy.md")).unwrap(),
        mixlayer::skill::PRIVACY
    );
    assert_eq!(
        fs::read_to_string(path.join("references/commands.md")).unwrap(),
        mixlayer::skill::COMMANDS
    );
    let ref_output = Command::new(cli)
        .args(["agent", "skill-ref"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.path())
        .output()
        .unwrap();
    assert!(ref_output.status.success());
    assert_eq!(ref_output.stdout, mixlayer::skill::COMMANDS.as_bytes());
}
