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
    let (arch, home) = parse_arch_and_home("Linux x86_64\n/home/test-user").unwrap();
    assert_eq!(arch, RemoteArch::LinuxX86_64);
    assert_eq!(home, "/home/test-user");
}

#[test]
fn parse_arch_and_home_trims_home_whitespace() {
    let (arch, home) = parse_arch_and_home("Darwin arm64\n/Users/test-user\n").unwrap();
    assert_eq!(arch, RemoteArch::DarwinArm64);
    assert_eq!(home, "/Users/test-user");
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
fn shell_quoting_escapes_single_quotes() {
    assert_eq!(shq("/home/me/proj"), "'/home/me/proj'");
    assert_eq!(shq("a'b"), "'a'\\''b'");
}

#[test]
fn parses_remote_directory_listing_and_sorts_names() {
    let listing =
        parse_directory_listing(b"/home/ubuntu/git\0zeta\0.alpha\0Clark Code Project\0").unwrap();
    assert_eq!(listing.path, "/home/ubuntu/git");
    assert_eq!(listing.parent.as_deref(), Some("/home/ubuntu"));
    assert_eq!(
        listing
            .directories
            .iter()
            .map(|directory| (directory.name.as_str(), directory.path.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (".alpha", "/home/ubuntu/git/.alpha"),
            ("Clark Code Project", "/home/ubuntu/git/Clark Code Project"),
            ("zeta", "/home/ubuntu/git/zeta"),
        ]
    );
}

#[test]
fn remote_directory_listing_handles_root() {
    let listing = parse_directory_listing(b"/\0home\0tmp\0").unwrap();
    assert_eq!(listing.parent, None);
    assert_eq!(listing.directories[0].path, "/home");
}

#[test]
fn remote_directory_listing_rejects_missing_absolute_path() {
    assert!(parse_directory_listing(b"relative\0child\0").is_err());
    assert!(parse_directory_listing(b"").is_err());
}
