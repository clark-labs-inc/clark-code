#[cfg(target_os = "macos")]
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if arguments == ["--self-test"] {
        computer_use::native_helper_self_test()
    } else {
        parse_service_arguments(&arguments).and_then(|(ipc_fd, control_fd, data_dir)| {
            computer_use::run_native_helper(ipc_fd, control_fd, data_dir)
        })
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn parse_service_arguments(
    arguments: &[String],
) -> Result<(i32, i32, std::path::PathBuf), computer_use::ComputerUseError> {
    let usage = || {
        computer_use::ComputerUseError::HelperProtocol(
            "usage: clark-computer-use-helper --ipc-fd <descriptor> --control-fd <descriptor> --data-dir <absolute-path>".to_string(),
        )
    };
    if arguments.len() != 6 {
        return Err(usage());
    }
    let mut ipc_fd = None;
    let mut control_fd = None;
    let mut data_dir = None;
    for pair in arguments.chunks_exact(2) {
        match pair[0].as_str() {
            "--ipc-fd" if ipc_fd.is_none() => {
                ipc_fd = Some(pair[1].parse::<i32>().map_err(|error| {
                    computer_use::ComputerUseError::HelperProtocol(format!(
                        "invalid IPC descriptor: {error}"
                    ))
                })?);
            }
            "--control-fd" if control_fd.is_none() => {
                control_fd = Some(pair[1].parse::<i32>().map_err(|error| {
                    computer_use::ComputerUseError::HelperProtocol(format!(
                        "invalid control descriptor: {error}"
                    ))
                })?);
            }
            "--data-dir" if data_dir.is_none() => {
                data_dir = Some(std::path::PathBuf::from(&pair[1]));
            }
            _ => return Err(usage()),
        }
    }
    match (ipc_fd, control_fd, data_dir) {
        (Some(ipc_fd), Some(control_fd), Some(data_dir)) => Ok((ipc_fd, control_fd, data_dir)),
        _ => Err(usage()),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("clark-computer-use-helper is only supported on macOS");
    std::process::exit(1);
}
