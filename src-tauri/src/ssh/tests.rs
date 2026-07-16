use super::*;

#[test]
fn parses_uname_to_arch() {
    assert_eq!(
        RemoteArch::from_uname("Linux x86_64\n").unwrap(),
        RemoteArch::LinuxX86_64
    );
    assert_eq!(
        RemoteArch::from_uname("Linux aarch64").unwrap(),
        RemoteArch::LinuxAarch64
    );
    assert_eq!(
        RemoteArch::from_uname("Darwin arm64").unwrap(),
        RemoteArch::DarwinArm64
    );
    assert!(RemoteArch::from_uname("Plan9 mips").is_err());
}

#[test]
fn parse_arch_and_home_splits_first_line_and_remainder() {
    let (arch, home) = parse_arch_and_home("Linux x86_64\n/home/stan").unwrap();
    assert_eq!(arch, RemoteArch::LinuxX86_64);
    assert_eq!(home, "/home/stan");
}

#[test]
fn parse_arch_and_home_trims_home_whitespace() {
    let (arch, home) = parse_arch_and_home("Darwin arm64\n/Users/stan\n").unwrap();
    assert_eq!(arch, RemoteArch::DarwinArm64);
    assert_eq!(home, "/Users/stan");
}

#[test]
fn parse_arch_and_home_errors_on_empty_home() {
    assert!(parse_arch_and_home("Linux x86_64\n").is_err());
    assert!(parse_arch_and_home("Linux x86_64\n   ").is_err());
}

#[test]
fn parse_arch_and_home_propagates_bad_arch() {
    assert!(parse_arch_and_home("Plan9 mips\n/home/x").is_err());
}

#[test]
fn arch_slugs_are_stable() {
    assert_eq!(RemoteArch::LinuxX86_64.slug(), "linux-x86_64");
    assert_eq!(RemoteArch::LinuxAarch64.slug(), "linux-aarch64");
    assert_eq!(RemoteArch::DarwinArm64.slug(), "darwin-aarch64");
}

#[test]
fn parses_server_url_line() {
    assert_eq!(
        parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:54321"),
        Some(54321)
    );
    assert_eq!(
        parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:54321\n"),
        Some(54321)
    );
    assert_eq!(parse_server_port("some other log line"), None);
    assert_eq!(
        parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:notaport"),
        None
    );
}

#[test]
fn shell_quoting_escapes_single_quotes() {
    assert_eq!(shq("/home/me/proj"), "'/home/me/proj'");
    assert_eq!(shq("a'b"), "'a'\\''b'");
}

#[test]
fn tokens_are_long_and_unique() {
    let a = new_token();
    let b = new_token();
    assert_ne!(a, b);
    assert_eq!(a.len(), 64);
}
