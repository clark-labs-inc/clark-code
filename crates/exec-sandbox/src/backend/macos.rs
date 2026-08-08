use std::ffi::OsString;
use std::path::Path;

use exec_core::ProcessSpec;

use crate::{NetworkPolicy, SandboxPolicy};

use super::{append_inner_command, original_parts};

pub(super) fn prepare(
    policy: &SandboxPolicy,
    helper: &Path,
    process: ProcessSpec,
) -> Result<ProcessSpec, String> {
    let env = process.env.clone();
    let (program, inner_args, cwd) = original_parts(process);
    let profile = compile_profile(policy)?;
    let mut args = vec![
        OsString::from("-p"),
        OsString::from(profile),
        OsString::from("--"),
    ];
    append_inner_command(&mut args, &program, inner_args);
    Ok(ProcessSpec {
        program: helper.to_path_buf(),
        args,
        cwd,
        env,
    })
}

pub(crate) fn compile_profile(policy: &SandboxPolicy) -> Result<String, String> {
    let mut sections = vec![
        "(version 1)".to_string(),
        "(allow default)".to_string(),
        "(deny file-write*)".to_string(),
        // Shells, Git, and PTY-backed commands need the standard character
        // devices without gaining a writable filesystem path.
        "(allow file-write-data (literal \"/dev/null\"))".to_string(),
        "(allow file-write* (regex #\"^/dev/fd/(1|2)$\"))".to_string(),
        "(allow file-read* file-write* file-ioctl (literal \"/dev/ptmx\"))".to_string(),
        "(allow file-read* file-write* file-ioctl (regex #\"^/dev/ttys[0-9]+$\"))".to_string(),
    ];
    for root in &policy.write_roots {
        sections.push(format!(
            "(allow file-write* (subpath \"{}\"))",
            seatbelt_string(root)
        ));
    }
    for root in &policy.deny_write {
        sections.push(format!(
            "(deny file-write* (subpath \"{}\"))",
            seatbelt_string(root)
        ));
    }
    for root in &policy.deny_read {
        sections.push(format!(
            "(deny file-read* (subpath \"{}\"))",
            seatbelt_string(root)
        ));
    }
    if policy.network == NetworkPolicy::Restricted {
        sections.push("(deny network*)".to_string());
    }
    Ok(sections.join("\n"))
}

fn seatbelt_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn compiler_never_interpolates_unescaped_quotes() {
        let policy = SandboxPolicy {
            read_roots: Vec::new(),
            write_roots: vec![PathBuf::from("/tmp/a\"b")],
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            network: NetworkPolicy::Restricted,
            process_temp_root: None,
        };
        let profile = compile_profile(&policy).unwrap();
        assert!(profile.contains("a\\\"b"));
        assert!(profile.contains("(deny network*)"));
    }
}
