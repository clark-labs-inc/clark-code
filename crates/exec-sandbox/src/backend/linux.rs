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
    let mut args = vec![
        OsString::from("--new-session"),
        OsString::from("--die-with-parent"),
        OsString::from("--ro-bind"),
        OsString::from("/"),
        OsString::from("/"),
        // The read-only host root also makes the inherited device node
        // read-only. Commands routinely open /dev/null for both reading and
        // writing (Git does this for disabled helpers), so overlay only that
        // non-persistent sink as a writable device without exposing the rest
        // of /dev.
        OsString::from("--dev-bind"),
        OsString::from("/dev/null"),
        OsString::from("/dev/null"),
        OsString::from("--unshare-user"),
        OsString::from("--unshare-pid"),
        OsString::from("--proc"),
        OsString::from("/proc"),
    ];
    if policy.network == NetworkPolicy::Restricted {
        args.push(OsString::from("--unshare-net"));
    }
    for root in &policy.write_roots {
        if root.exists() {
            args.push(OsString::from("--bind"));
            args.push(root.as_os_str().to_os_string());
            args.push(root.as_os_str().to_os_string());
        }
    }
    for root in &policy.deny_write {
        if root.exists() {
            args.push(OsString::from("--ro-bind"));
            args.push(root.as_os_str().to_os_string());
            args.push(root.as_os_str().to_os_string());
        }
    }
    for root in &policy.deny_read {
        if root.exists() {
            args.push(OsString::from(if root.is_dir() {
                "--tmpfs"
            } else {
                "--ro-bind"
            }));
            if root.is_file() {
                args.push(OsString::from("/dev/null"));
            }
            args.push(root.as_os_str().to_os_string());
        }
    }
    args.push(OsString::from("--chdir"));
    args.push(cwd.as_os_str().to_os_string());
    args.push(OsString::from("--"));
    append_inner_command(&mut args, &program, inner_args);
    Ok(ProcessSpec {
        program: helper.to_path_buf(),
        args,
        cwd,
        env,
    })
}
