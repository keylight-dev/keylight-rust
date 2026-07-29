use assert_cmd::Command;
#[test]
fn help_lists_subcommands() {
    Command::cargo_bin("keylight-demo")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}
#[test]
fn missing_required_tenant_fails() {
    Command::cargo_bin("keylight-demo")
        .unwrap()
        .args(["status"])
        .env_remove("KEYLIGHT_TENANT")
        .assert()
        .failure();
}
