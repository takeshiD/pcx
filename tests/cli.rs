use std::process::Command;

#[test]
fn help_identifies_the_product_and_its_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("--help")
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Inspect and reduce point-cloud recordings"));
    assert!(stdout.contains("under active development"));
}

#[test]
fn version_matches_the_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("--version")
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version should be UTF-8"),
        format!("pcx {}\n", env!("CARGO_PKG_VERSION"))
    );
}
