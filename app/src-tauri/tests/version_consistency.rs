// The release process bumps the version in three files at once
// (docs/deployment.md step 3). This test fails if they drift apart.
use std::fs;

fn package_json_version() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/package.json");
    let text = fs::read_to_string(path).expect("app/package.json");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse app/package.json");
    value["version"]
        .as_str()
        .expect("package.json has a version")
        .to_string()
}

fn cargo_version() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = fs::read_to_string(path).expect("Cargo.toml");
    text.lines()
        .find(|line| line.starts_with("version = "))
        .expect("Cargo.toml [package] version")
        .split('"')
        .nth(1)
        .expect("quoted version")
        .to_string()
}

fn tauri_conf_version() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let text = fs::read_to_string(path).expect("tauri.conf.json");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse tauri.conf.json");
    value["version"]
        .as_str()
        .expect("tauri.conf.json has a version")
        .to_string()
}

#[test]
fn versions_match_across_manifests() {
    let v = cargo_version();
    assert_eq!(v, package_json_version(), "Cargo.toml vs package.json");
    assert_eq!(v, tauri_conf_version(), "Cargo.toml vs tauri.conf.json");
}
