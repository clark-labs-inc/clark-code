#[path = "skill_experience_benchmark/fixture.rs"]
mod fixture;
#[path = "skill_experience_benchmark/journey.rs"]
mod journey;
#[path = "skill_experience_benchmark/model.rs"]
mod model;
#[path = "skill_experience_benchmark/model_server.rs"]
mod model_server;
#[path = "skill_experience_benchmark/provider_harness.rs"]
mod provider_harness;

use std::path::{Path, PathBuf};

use model::{DynError, Recorder};
use uuid::Uuid;

#[derive(Debug)]
struct Options {
    source: Source,
    output: PathBuf,
}

#[derive(Debug)]
enum Source {
    Checkout(PathBuf),
    Synthetic,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, DynError> {
        let mut superpowers = None;
        let mut synthetic = false;
        let mut output = None;
        let mut args = args.peekable();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--superpowers" => superpowers = Some(next_path(&mut args, "--superpowers")?),
                "--synthetic" => synthetic = true,
                "--out" => output = Some(next_path(&mut args, "--out")?),
                "--help" | "-h" => {
                    println!(
                        "Usage: cargo run -p provider-local --example skill_experience_benchmark -- \\\n                         [--superpowers /path/to/obra/superpowers | --synthetic] \\\n                         [--out /path/to/new-output]\n\n\
                         Runs a deterministic, no-credits simulation from isolated empty local and \
                         remote user homes. Use --synthetic for the self-contained CI fixture. \
                         The output directory must not already exist."
                    );
                    std::process::exit(0);
                }
                other => return Err(model::error(format!("unknown argument `{other}`"))),
            }
        }
        if synthetic && superpowers.is_some() {
            return Err(model::error(
                "--synthetic and --superpowers are mutually exclusive",
            ));
        }
        let source = if synthetic {
            Source::Synthetic
        } else {
            Source::Checkout(resolve_superpowers(superpowers)?)
        };
        let output = output.unwrap_or_else(|| {
            PathBuf::from("target/skill-experience-benchmark").join(Uuid::new_v4().to_string())
        });
        let output = if output.is_absolute() {
            output
        } else {
            std::env::current_dir()?.join(output)
        };
        Ok(Self { source, output })
    }
}

fn next_path(args: &mut impl Iterator<Item = String>, option: &str) -> Result<PathBuf, DynError> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| model::error(format!("{option} requires a path")))
}

fn resolve_superpowers(explicit: Option<PathBuf>) -> Result<PathBuf, DynError> {
    let candidates = explicit
        .into_iter()
        .chain(std::env::var_os("AGENT_SUPERPOWERS_FIXTURE").map(PathBuf::from))
        .chain(
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .map(|root| root.join(".tmp/plugins/plugins/superpowers")),
        );
    for candidate in candidates {
        if candidate.join("skills/brainstorming/SKILL.md").is_file() {
            return candidate
                .canonicalize()
                .map_err(|error| model::error(format!("canonicalize source: {error}")));
        }
    }
    Err(model::error(
        "could not find an obra/superpowers checkout; pass --superpowers /path/to/repository",
    ))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DynError> {
    let options = Options::parse(std::env::args().skip(1))?;
    create_output(&options.output)?;
    let source = match options.source {
        Source::Checkout(path) => path,
        Source::Synthetic => {
            let path = options.output.join("fixtures/source-superpowers");
            fixture::create_synthetic_superpowers(&path)?;
            path
        }
    };
    let source_digest = fixture::tree_digest(&source)?;
    let mut recorder = Recorder::new(&options.output, &source, &source_digest);
    let result = journey::run(&source, &options.output, &mut recorder).await;
    recorder.write_artifacts(result.as_ref().err().map(ToString::to_string))?;

    println!("Report: {}", options.output.join("report.md").display());
    println!("Receipt: {}", options.output.join("receipt.json").display());
    println!(
        "Result: {} ({} steps, no live model calls)",
        if result.is_ok() { "PASS" } else { "FAIL" },
        recorder.steps().len()
    );
    result
}

fn create_output(output: &Path) -> Result<(), DynError> {
    if output.exists() {
        return Err(model::error(format!(
            "refusing to overwrite benchmark output {}",
            output.display()
        )));
    }
    std::fs::create_dir_all(output)
        .map_err(|error| model::error(format!("create {}: {error}", output.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn copy_tree_materializes_internal_file_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("benchmark test root");
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        std::fs::create_dir_all(&source).expect("source directory");
        std::fs::write(source.join("CLAUDE.md"), "fixture instructions\n")
            .expect("fixture instructions");
        symlink("CLAUDE.md", source.join("AGENTS.md")).expect("fixture symlink");

        fixture::copy_tree(&source, &destination).expect("copy source");

        assert_eq!(
            std::fs::read_to_string(destination.join("AGENTS.md")).expect("copied instructions"),
            "fixture instructions\n"
        );
        assert!(!std::fs::symlink_metadata(destination.join("AGENTS.md"))
            .expect("copied metadata")
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn copy_tree_rejects_symlink_outside_source() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("benchmark test root");
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        std::fs::create_dir_all(&source).expect("source directory");
        let outside = root.path().join("outside.md");
        std::fs::write(&outside, "outside\n").expect("outside fixture");
        symlink(&outside, source.join("escape.md")).expect("escaping symlink");

        let error = fixture::copy_tree(&source, &destination).expect_err("reject escaping symlink");

        assert!(error.to_string().contains("escapes source"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synthetic_empty_user_journey_passes_every_contract() {
        let root = tempfile::tempdir().expect("benchmark test root");
        let source = root.path().join("synthetic-superpowers");
        let output = root.path().join("output");
        fixture::create_synthetic_superpowers(&source).expect("synthetic fixture");
        create_output(&output).expect("benchmark output");
        let digest = fixture::tree_digest(&source).expect("source digest");
        let mut recorder = Recorder::new(&output, &source, &digest);

        let result = journey::run(&source, &output, &mut recorder).await;
        recorder
            .write_artifacts(result.as_ref().err().map(ToString::to_string))
            .expect("benchmark artifacts");

        result.expect("full synthetic journey");
        assert_eq!(recorder.steps().len(), 10);
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("receipt.json")).expect("receipt"))
                .expect("receipt JSON");
        assert_eq!(receipt["status"], "passed");
        assert_eq!(receipt["liveModelCalls"], 0);
    }
}
