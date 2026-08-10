use std::env;
use std::fs;
use std::path::PathBuf;

const INPUTS: &[(&str, &str)] = &[
    ("DESKTOP_COMPUTER_USE_PROD_APP_ID", "org.agentdesktop.app"),
    (
        "DESKTOP_COMPUTER_USE_PROD_HELPER_ID",
        "org.agentdesktop.computer-use",
    ),
    ("DESKTOP_COMPUTER_USE_PROD_TEAM_ID", "AGENTPROD"),
    (
        "DESKTOP_COMPUTER_USE_DEV_APP_ID",
        "org.agentdesktop.app.dev",
    ),
    (
        "DESKTOP_COMPUTER_USE_DEV_HELPER_ID",
        "org.agentdesktop.computer-use.dev",
    ),
    ("DESKTOP_COMPUTER_USE_DEV_TEAM_ID", "AGENTDEV"),
];

fn value(name: &str, fallback: &str) -> String {
    println!("cargo:rerun-if-env-changed={name}");
    let value = env::var(name).unwrap_or_else(|_| fallback.to_string());
    assert!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')),
        "{name} contains unsupported code-requirement characters"
    );
    value
}

fn path_value(name: &str, fallback: &str) -> String {
    println!("cargo:rerun-if-env-changed={name}");
    let value = env::var(name).unwrap_or_else(|_| fallback.to_string());
    assert!(
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b' ')
            }),
        "{name} contains unsupported path characters"
    );
    value
}

fn main() {
    let values = INPUTS
        .iter()
        .map(|(name, fallback)| value(name, fallback))
        .collect::<Vec<_>>();
    for ((name, _), resolved) in INPUTS.iter().zip(&values) {
        println!("cargo:rustc-env={name}={resolved}");
    }
    println!(
        "cargo:rustc-env=DESKTOP_COMPUTER_USE_DATA_NAMESPACE={}",
        value("DESKTOP_COMPUTER_USE_DATA_NAMESPACE", ".agent-desktop")
    );
    println!(
        "cargo:rustc-env=DESKTOP_COMPUTER_USE_MAC_SUPPORT_NAME={}",
        path_value("DESKTOP_COMPUTER_USE_MAC_SUPPORT_NAME", "Clark Code")
    );
    println!(
        "cargo:rustc-env=DESKTOP_COMPUTER_USE_MAC_HELPER_APP={}",
        path_value(
            "DESKTOP_COMPUTER_USE_MAC_HELPER_APP",
            "Agent Computer Use.app"
        )
    );
    println!(
        "cargo:rustc-env=DESKTOP_COMPUTER_USE_HELPER_EXECUTABLE={}",
        path_value(
            "DESKTOP_COMPUTER_USE_HELPER_EXECUTABLE",
            "agent-computer-use-helper"
        )
    );
    let generated = format!(
        r##"#[allow(dead_code)]
const CLIENT_SIGNING_REQUIREMENT: &str = r#"
(
  identifier "{}"
  and anchor apple generic
  and certificate leaf[subject.OU] = "{}"
)
or
(
  identifier "{}"
  and anchor apple generic
  and certificate leaf[subject.OU] = "{}"
)
"#;

#[allow(dead_code)]
const SERVICE_SIGNING_REQUIREMENT: &str = r#"
(
  identifier "{}"
  and anchor apple generic
  and certificate leaf[subject.OU] = "{}"
)
or
(
  identifier "{}"
  and anchor apple generic
  and certificate leaf[subject.OU] = "{}"
)
"#;
"##,
        values[0], values[2], values[3], values[5], values[1], values[2], values[4], values[5]
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("signing.rs");
    fs::write(output, generated).expect("write computer-use signing policy");
}
