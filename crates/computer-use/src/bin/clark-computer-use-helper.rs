#[cfg(target_os = "macos")]
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if arguments == ["--self-test"] {
        computer_use::native_helper_self_test()
    } else {
        parse_service_arguments(&arguments).and_then(|(socket_path, data_dir)| {
            computer_use::run_native_helper(socket_path, data_dir)
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
) -> Result<(std::path::PathBuf, std::path::PathBuf), computer_use::ComputerUseError> {
    let usage = || {
        computer_use::ComputerUseError::HelperProtocol(
            "usage: clark-computer-use-helper --socket <absolute-path> --data-dir <absolute-path>"
                .to_string(),
        )
    };
    if arguments.len() != 4 {
        return Err(usage());
    }
    let mut socket_path = None;
    let mut data_dir = None;
    for pair in arguments.chunks_exact(2) {
        match pair[0].as_str() {
            "--socket" if socket_path.is_none() => {
                socket_path = Some(std::path::PathBuf::from(&pair[1]));
            }
            "--data-dir" if data_dir.is_none() => {
                data_dir = Some(std::path::PathBuf::from(&pair[1]));
            }
            _ => return Err(usage()),
        }
    }
    match (socket_path, data_dir) {
        (Some(socket_path), Some(data_dir))
            if socket_path.is_absolute() && data_dir.is_absolute() =>
        {
            Ok((socket_path, data_dir))
        }
        _ => Err(usage()),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    if let Err(error) = portable_main() {
        eprintln!("clark-computer-use service failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn portable_main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments == ["--self-test"] {
        computer_use::portable_service_self_test()?;
        return Ok(());
    }
    if arguments.len() != 6
        || arguments[0] != "--socket-name"
        || arguments[2] != "--data-dir"
        || arguments[4] != "--client-pid"
    {
        return Err(
            "usage: clark-computer-use-helper --socket-name <name> --data-dir <absolute-path> --client-pid <pid>"
                .into(),
        );
    }
    let data_dir = std::path::PathBuf::from(&arguments[3]);
    if !data_dir.is_absolute() {
        return Err("--data-dir must be absolute".into());
    }
    let client_pid = arguments[5].parse::<u32>()?;
    computer_use::run_portable_service(arguments[1].clone(), data_dir, client_pid)?;
    Ok(())
}
