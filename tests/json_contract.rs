use std::{fs, path::Path, process::Command};

use serde_json::Value;

const VALID_SOURCE: &str = "tests/fixtures/valid/pointcloud2.mcap";
const INVALID_SOURCE: &str = "tests/fixtures/malformed/mcap-leading-magic-must-match.mcap";

fn run(command: &str, source: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([command, source, "--json"])
        .output()
        .expect("pcx should start")
}

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("machine output should be JSON")
}

fn golden(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/json/v1")
        .join(name);
    parse(&fs::read(path).expect("reviewed golden should be readable"))
}

fn normalize_error_message(value: &mut Value) {
    value["error"]["message"] = Value::String("<normalized-message>".to_owned());
}

#[test]
fn every_successful_json_command_matches_its_reviewed_golden() {
    for command in ["info", "topics"] {
        let output = run(command, VALID_SOURCE);
        assert!(output.status.success(), "{command} should succeed");
        assert!(output.stderr.is_empty(), "{command} stderr should be empty");
        assert_eq!(parse(&output.stdout), golden(&format!("{command}.json")));
    }
}

#[test]
fn every_json_command_failure_matches_its_reviewed_normalized_golden() {
    for command in ["info", "topics"] {
        let output = run(command, INVALID_SOURCE);
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stdout.is_empty(), "{command} stdout should be empty");
        let mut actual = parse(&output.stderr);
        assert!(
            actual["error"]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "the real diagnostic must not be empty"
        );
        normalize_error_message(&mut actual);
        assert_eq!(actual, golden(&format!("{command}-error.json")));
    }
}
