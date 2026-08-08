fn main() {
    let code = exec_sandbox_windows::runner_main(std::env::args_os().skip(1));
    std::process::exit(code);
}
