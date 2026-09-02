//! Uses synthetic files only; never reads Messages, requests macOS privacy
//! access, or opens the real Messages database.
#![cfg(target_os = "macos")]
use exec_core::ProcessSpec;
use exec_sandbox::{SandboxManager, SandboxPolicy};

#[test]
fn sandboxed_process_cannot_read_or_write_messages_even_through_symlink() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let ordinary = root.join("ordinary.txt");
    std::fs::write(&ordinary, "ordinary fixture").unwrap();
    let manager =
        SandboxManager::current(SandboxPolicy::workspace_write(root.clone(), vec![])).unwrap();
    let process = manager
        .prepare_process(ProcessSpec::argv("/bin/cat", &root).args([&ordinary]))
        .unwrap();
    let output = std::process::Command::new(process.program)
        .args(process.args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, b"ordinary fixture");
    for relative in ["Library/Messages/chat.db"] {
        let private = root.join(relative);
        std::fs::create_dir_all(private.parent().unwrap()).unwrap();
        std::fs::write(&private, "PRIVATE_FIXTURE_SENTINEL").unwrap();
        let alias = root.join("messages-alias");
        std::os::unix::fs::symlink(&private, &alias).unwrap();
        let manager =
            SandboxManager::current(SandboxPolicy::workspace_write(root.clone(), vec![])).unwrap();
        for target in [&private, &alias] {
            let process = manager
                .prepare_process(ProcessSpec::argv("/bin/cat", &root).args([target]))
                .unwrap();
            let output = std::process::Command::new(process.program)
                .args(process.args)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(!String::from_utf8_lossy(&output.stderr).contains("syntax error"));
            let process = manager
                .prepare_process(ProcessSpec::argv("/usr/bin/tee", &root).args([target]))
                .unwrap();
            let mut child = std::process::Command::new(process.program)
                .args(process.args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"attempted overwrite")
                .unwrap();
            assert!(!child.wait_with_output().unwrap().status.success());
            assert_eq!(
                std::fs::read_to_string(&private).unwrap(),
                "PRIVATE_FIXTURE_SENTINEL"
            );
        }
    }
}
