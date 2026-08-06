use std::path::PathBuf;

use scout_capability_census::{run_census, CensusConfig, CensusLimits};

fn main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(ParseResult::Help) => {
            print_help();
        }
        Ok(ParseResult::Run { config, pretty }) => match run_census(config) {
            Ok(receipt) => {
                let encoded = if pretty {
                    serde_json::to_string_pretty(&receipt)
                } else {
                    serde_json::to_string(&receipt)
                };
                match encoded {
                    Ok(encoded) => println!("{encoded}"),
                    Err(error) => fail(&format!("receipt serialization failed: {error}")),
                }
            }
            Err(error) => fail(&error.to_string()),
        },
        Err(error) => {
            eprintln!("error: {error}");
            print_help();
            std::process::exit(2);
        }
    }
}

enum ParseResult {
    Help,
    Run { config: CensusConfig, pretty: bool },
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<ParseResult, String> {
    let mut limits = CensusLimits::default();
    let mut roots = Vec::new();
    let mut pretty = false;
    let mut args = args.peekable();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(ParseResult::Help),
            "--pretty" => pretty = true,
            "--root" => roots.push(PathBuf::from(next_value(&mut args, "--root")?)),
            "--max-depth" => {
                limits.max_depth = parse_usize(&next_value(&mut args, "--max-depth")?)?
            }
            "--max-directories" => {
                limits.max_directories = parse_usize(&next_value(&mut args, "--max-directories")?)?
            }
            "--max-files" => {
                limits.max_dotenv_files = parse_usize(&next_value(&mut args, "--max-files")?)?
            }
            "--max-bytes" => {
                limits.max_total_bytes = parse_u64(&next_value(&mut args, "--max-bytes")?)?
            }
            "--max-file-bytes" => {
                limits.max_file_bytes = parse_u64(&next_value(&mut args, "--max-file-bytes")?)?
            }
            "--max-keys-per-file" => {
                limits.max_keys_per_file =
                    parse_usize(&next_value(&mut args, "--max-keys-per-file")?)?
            }
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }
    Ok(ParseResult::Run {
        config: CensusConfig {
            scan_roots: roots,
            limits,
        },
        pretty,
    })
}

fn next_value(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    option: &str,
) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {option}"))
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("`{value}` is not a valid non-negative integer"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("`{value}` is not a valid non-negative integer"))
}

fn fail(message: &str) -> ! {
    eprintln!("Scout capability census failed: {message}");
    std::process::exit(1);
}

fn print_help() {
    println!(
        "Secret-safe, read-only Scout capability census

Usage:
  scout_capability_census --root PATH [--root PATH ...] [OPTIONS]

Options:
  --root PATH             Explicit workspace/component root (repeatable)
  --max-depth N           Maximum directory depth per root (default: 8)
  --max-directories N     Maximum directories across all roots (default: 4096)
  --max-files N           Maximum dotenv files (default: 128)
  --max-bytes N           Maximum total dotenv bytes read (default: 8388608)
  --max-file-bytes N      Maximum bytes read from one dotenv file (default: 1048576)
  --max-keys-per-file N   Maximum emitted key names per dotenv file (default: 512)
  --pretty                Pretty-print the JSON receipt
  -h, --help              Show this help

The executable writes only JSON to stdout. It never executes discovered tools,
emits environment/dotenv values, crawls outside explicit roots, or follows
symlink entries. Redirect stdout if a durable receipt is required."
    );
}
