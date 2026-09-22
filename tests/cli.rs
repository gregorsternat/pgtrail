use std::process::{Command, Stdio};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgtrail"));
    command.stdin(Stdio::null());
    command.env_remove("PGTRAIL_DATABASE_URL");
    command.env_remove("PGTRAIL_STORE");
    command
}

#[test]
fn help_works_without_a_terminal() -> std::io::Result<()> {
    let output = command().arg("--help").output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: pgtrail"));
    assert!(stdout.contains("--demo"));
    assert!(stdout.contains("offline comparisons"));
    Ok(())
}

#[test]
fn version_matches_package_metadata() -> std::io::Result<()> {
    let output = command().arg("--version").output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("pgtrail ", env!("CARGO_PKG_VERSION"))
    );
    Ok(())
}

#[test]
fn unknown_arguments_fail_with_usage() -> std::io::Result<()> {
    let output = command().arg("--does-not-exist").output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    Ok(())
}

#[test]
fn non_interactive_start_fails_without_terminal_escape_sequences() -> std::io::Result<()> {
    let output = command().output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an interactive terminal"));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.contains(&0x1b));
    Ok(())
}

#[test]
fn demo_capture_restart_export_compare_and_delete() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = directory.path().join("history.sqlite3");
    for label in ["Before", "After"] {
        let output = command()
            .arg("--store")
            .arg(&store)
            .args(["--demo", "capture", "--label", label])
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Saved capture #"));
    }
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["snapshots", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let list: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(list.as_array().map(Vec::len), Some(2));

    // Offline commands must work even if the user's live URL is invalid.
    let output = command()
        .env("PGTRAIL_DATABASE_URL", "invalid secret URL")
        .arg("--store")
        .arg(&store)
        .args(["show", "1", "--format", "json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(snapshot["schema_version"], 2);
    assert_eq!(snapshot["source"]["database"], "demo_shop");
    for session in snapshot["activity"]["data"]
        .as_array()
        .ok_or("missing activity")?
    {
        assert!(session["query"].is_null());
    }

    let export = directory.path().join("incident.md");
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["compare", "1", "2", "--output"])
        .arg(&export)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let markdown = std::fs::read_to_string(&export)?;
    assert!(markdown.contains("demo\\_shop"));
    assert!(markdown.contains("Statement") || markdown.contains("statement"));
    // Existing exports are never silently replaced.
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["show", "1", "--output"])
        .arg(&export)
        .output()?;
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(&export)?, markdown);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&export)?.permissions().mode() & 0o777,
            0o600
        );
    }
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["delete", "1"])
        .output()?;
    assert!(output.status.success());
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["show", "1"])
        .output()?;
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn sql_text_requires_explicit_opt_in() -> Result<(), Box<dyn std::error::Error>> {
    let output = command()
        .args([
            "--demo",
            "--include-query-text",
            "check",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(snapshot["activity"]["data"][0]["query"].as_str().is_some());
    Ok(())
}

#[test]
fn connection_errors_never_echo_credentials() -> std::io::Result<()> {
    for url in [
        "postgres://user:do-not-print-this@[invalid",
        "definitely-not-a-url-do-not-print-this",
    ] {
        let output = command()
            .env("PGTRAIL_DATABASE_URL", url)
            .arg("check")
            .output()?;
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!combined.contains("do-not-print-this"));
        assert!(!combined.contains(url));
    }
    Ok(())
}

#[test]
fn argument_bounds_and_missing_configuration_are_clear() -> std::io::Result<()> {
    for args in [["--refresh", "0"], ["--timeout", "121"], ["show", "0"]] {
        let output = command().args(args).output()?;
        assert_eq!(output.status.code(), Some(2));
    }
    let output = command().arg("check").output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PGTRAIL_DATABASE_URL"));
    Ok(())
}

fn successful(
    args: &[&str],
    store: &std::path::Path,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let output = command().arg("--store").arg(store).args(args).output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

#[test]
fn incident_dossier_survives_restart_and_closed_capture_is_atomic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = directory.path().join("incidents.sqlite3");
    successful(&["incident", "create", "Checkout latency"], &store)?;
    successful(
        &[
            "--demo",
            "--incident",
            "1",
            "capture",
            "--label",
            "Blocked checkout",
        ],
        &store,
    )?;
    successful(
        &[
            "incident",
            "note",
            "1",
            "Investigating the transaction owner",
        ],
        &store,
    )?;
    successful(
        &[
            "annotate",
            "1",
            "--note",
            "Evidence <script> stays plain",
            "--label",
            "Before operator action",
        ],
        &store,
    )?;
    let shown: serde_json::Value =
        serde_json::from_slice(&successful(&["show", "1", "--format", "json"], &store)?)?;
    assert_eq!(shown["capture"]["label"], "Before operator action");
    assert_eq!(shown["capture"]["note"], "Evidence <script> stays plain");
    successful(&["incident", "close", "1"], &store)?;
    let rejected = command()
        .arg("--store")
        .arg(&store)
        .args(["--demo", "--incident", "1", "capture"])
        .output()?;
    assert!(!rejected.status.success());
    let list: serde_json::Value =
        serde_json::from_slice(&successful(&["snapshots", "--json"], &store)?)?;
    assert_eq!(
        list.as_array().map(Vec::len),
        Some(1),
        "failed incident attachment must not save an orphan capture"
    );
    successful(&["incident", "reopen", "1"], &store)?;
    successful(
        &[
            "--demo",
            "--incident",
            "1",
            "capture",
            "--label",
            "Follow up",
        ],
        &store,
    )?;
    let dossier: serde_json::Value = serde_json::from_slice(&successful(
        &["incident", "show", "1", "--format", "json"],
        &store,
    )?)?;
    assert_eq!(
        dossier["incident"]["captures"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        dossier["incident"]["notes"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        dossier["incident"]["capture_notes"]["1"],
        "Evidence <script> stays plain"
    );
    let markdown = String::from_utf8(successful(&["incident", "show", "1"], &store)?)?;
    assert!(markdown.contains("Checkout latency"));
    assert!(markdown.contains("Investigating the transaction owner"));
    assert!(!markdown.contains("<script>"));
    assert!(markdown.contains("Before operator action"));
    successful(&["incident", "detach", "1", "2"], &store)?;
    let list: serde_json::Value =
        serde_json::from_slice(&successful(&["snapshots", "--json"], &store)?)?;
    assert_eq!(
        list.as_array().map(Vec::len),
        Some(2),
        "detaching evidence preserves the saved capture"
    );
    Ok(())
}

#[test]
fn bounded_recording_and_threshold_diagnostics_work_headlessly()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = directory.path().join("record.sqlite3");
    successful(
        &[
            "--demo",
            "record",
            "--count",
            "2",
            "--interval",
            "1",
            "--label",
            "Investigation",
        ],
        &store,
    )?;
    let list: serde_json::Value =
        serde_json::from_slice(&successful(&["snapshots", "--json"], &store)?)?;
    assert_eq!(list.as_array().map(Vec::len), Some(2));
    let output = command()
        .args([
            "--demo",
            "diagnose",
            "--sample-seconds",
            "1",
            "--format",
            "json",
            "--fail-on",
            "warning",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "synthetic incident must cross warning threshold"
    );
    let diagnostic: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(
        diagnostic["analysis"]["rates"]["elapsed_seconds"]
            .as_f64()
            .is_some_and(|v| v > 0.0)
    );
    assert!(
        !diagnostic["analysis"]["findings"]
            .as_array()
            .ok_or("missing findings")?
            .is_empty()
    );
    for args in [
        vec!["record", "--count", "0"],
        vec!["record", "--interval", "0"],
        vec!["diagnose", "--sample-seconds", "121"],
        vec!["annotate", "1"],
    ] {
        assert_eq!(command().args(args).output()?.status.code(), Some(2));
    }
    Ok(())
}

#[test]
fn profile_management_keeps_secrets_out_of_configuration_and_errors()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let profiles = directory.path().canonicalize()?.join("profiles.json");
    let secret = "test-password-that-must-never-be-stored";
    let output = command()
        .arg("--profiles-file")
        .arg(&profiles)
        .env("PGTRAIL_CLI_TEST_PASSWORD", secret)
        .args([
            "profile",
            "add",
            "local",
            "--host",
            "127.0.0.1",
            "--database",
            "app",
            "--user",
            "monitor",
            "--password-env",
            "PGTRAIL_CLI_TEST_PASSWORD",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = std::fs::read_to_string(&profiles)?;
    assert!(!content.contains(secret));
    assert!(content.contains("verify-full"));
    assert!(content.contains("PGTRAIL_CLI_TEST_PASSWORD"));
    let output = command()
        .arg("--profiles-file")
        .arg(&profiles)
        .env_remove("PGTRAIL_CLI_TEST_PASSWORD")
        .args(["--profile", "local", "check"])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PGTRAIL_CLI_TEST_PASSWORD"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    let list = command()
        .arg("--profiles-file")
        .arg(&profiles)
        .args(["profile", "list"])
        .output()?;
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("local"));
    let remove = command()
        .arg("--profiles-file")
        .arg(&profiles)
        .args(["profile", "remove", "local"])
        .output()?;
    assert!(remove.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&profiles)?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(())
}
