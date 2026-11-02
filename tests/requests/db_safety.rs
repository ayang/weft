#[test]
fn test_config_never_reads_the_development_database_url() {
    let config = include_str!("../../config/test.yaml");
    assert!(
        config.contains("sqlite://weft_test.sqlite?mode=rwc"),
        "test DB must be explicitly weft_test.sqlite"
    );
    assert!(
        !config.contains("get_env(name=\"DATABASE_URL\""),
        "test config must not inherit DATABASE_URL from the developer shell"
    );
}
