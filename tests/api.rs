use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn cli(home: &Path, args: &[&str], input: Option<&str>, key: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mixlayer"));
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("MIXLAYER_HOME", home.join("mixlayer"))
        .env("MIXLAYER_BASE_URL", "http://127.0.0.1:9/v1")
        .env("MIXLAYER_NO_AGENT_DETECT", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(key) = key {
        command.env("MIXLAYER_API_KEY", key);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(text) = input {
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
fn set_token_reads_stdin_without_echo_and_writes_private_file() {
    let home = tempfile::tempdir().unwrap();
    let dummy = "mlk-test-stdin-key-1234567890";
    let output = cli(
        home.path(),
        &["auth", "set-token", "--json"],
        Some(&format!("{dummy}\n")),
        None,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!output.stdout.windows(dummy.len()).any(|w| w == dummy.as_bytes()));
    assert!(!output.stderr.windows(dummy.len()).any(|w| w == dummy.as_bytes()));
    let credentials = home.path().join("mixlayer/data/credentials.toml");
    let config = home.path().join("mixlayer/config/config.toml");
    assert!(std::fs::read_to_string(&credentials).unwrap().contains(dummy));
    assert!(!std::fs::read_to_string(&config).unwrap().contains(dummy));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&credentials).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let status = cli(home.path(), &["auth", "status", "--json"], None, None);
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["authenticated"], true);
    assert_eq!(value["source"], "config");
    let logout = cli(home.path(), &["auth", "logout", "--json"], None, None);
    assert!(logout.status.success());
    assert!(!credentials.exists());
}

#[test]
fn set_token_rejects_spaces_and_env_key_takes_priority() {
    let home = tempfile::tempdir().unwrap();
    let bad = cli(home.path(), &["auth", "set-token"], Some("this is a sentence"), None);
    assert_eq!(bad.status.code(), Some(2));
    let saved = cli(home.path(), &["auth", "set-token"], Some("mlk-file-dummy"), None);
    assert!(saved.status.success());
    let status = cli(home.path(), &["auth", "status", "--json"], None, Some("mlk-env-dummy"));
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["source"], "env:MIXLAYER_API_KEY");
    assert!(!String::from_utf8_lossy(&status.stdout).contains("mlk-env-dummy"));
}

#[test]
fn doctor_reports_missing_auth_and_paths_as_json() {
    let home = tempfile::tempdir().unwrap();
    let output = cli(home.path(), &["doctor", "--json", "--timeout", "40ms"], None, None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["auth"], "not configured");
    assert_eq!(value["paths"]["status"], "ok");
    assert_eq!(
        std::path::Path::new(value["cache"]["db_path"].as_str().unwrap()).file_name(),
        Some(std::ffi::OsStr::new("data.db"))
    );
    assert!(value["api"].as_str().unwrap().starts_with("unreachable:"));
}
