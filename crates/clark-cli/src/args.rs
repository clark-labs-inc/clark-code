use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "clark",
    version,
    about = "Clark's human-facing terminal agent",
    long_about = "Clark works in your current project from a terminal. Run it with no arguments for the interactive TUI, or choose Scout, Security, Scientist, or RSI directly."
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Select a paid Clark organization only when this account has more than one eligible workspace.
    #[arg(long, global = true, value_name = "ORGANIZATION_ID")]
    pub organization: Option<String>,

    /// Select a Scout cartography workspace only when more than one is available.
    #[arg(long, global = true, value_name = "WORKSPACE_ID")]
    pub workspace: Option<String>,

    /// Reopen one account-scoped Clark Cloud conversation by id.
    #[arg(long, global = true, value_name = "CONVERSATION_ID")]
    pub conversation: Option<String>,

    #[arg(long, global = true, conflicts_with = "json")]
    pub plain: bool,

    #[arg(long, global = true, conflicts_with = "plain")]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in without installing or opening Clark Desktop.
    Login(LoginArgs),
    /// Inspect or manage the stored Clark credential.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Remove this machine's stored Clark credential.
    Logout,
    /// Install the latest verified Clark CLI and paired specialist worker.
    Update(UpdateArgs),
    /// Check credentials, runtime dependencies, and specialist availability.
    Doctor,
    /// Map systems from bounded, evidence-backed investigation.
    Scout(ScoutArgs),
    /// Find vulnerabilities and produce evidence-backed findings.
    Security(SecurityArgs),
    /// Run preregistered experiments with cloud-synchronized artifacts.
    Scientist(ScientistArgs),
    /// Research and improve evaluations and evaluation worlds.
    Rsi(RsiArgs),
    /// Start the general Clark coding agent explicitly.
    Code(PromptArgs),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum LoginMethod {
    Browser,
    DeviceCode,
    ApiKey,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Browser is the default on a local terminal; device code is recommended over SSH.
    #[arg(long, value_enum)]
    pub method: Option<LoginMethod>,

    /// Read an API key from stdin. The key is never accepted as a command-line value.
    #[arg(long, conflicts_with = "method")]
    pub api_key: bool,

    /// Use a one-time code that can be approved from another device.
    #[arg(long, conflicts_with_all = ["method", "api_key"])]
    pub device_code: bool,
}

#[derive(Debug, Args, Default)]
pub struct UpdateArgs {
    /// Install an exact Clark version instead of the latest stable release.
    #[arg(long, value_name = "VERSION")]
    pub release: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    Status,
}

#[derive(Debug, Args, Default)]
pub struct PromptArgs {
    /// Run one turn and print the result instead of opening the TUI.
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct ScoutArgs {
    /// Create a Scout cartography workspace in the selected paid organization.
    #[arg(long, value_name = "NAME")]
    pub create_workspace: Option<String>,
    /// Run one turn and print the result instead of opening the TUI.
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SecurityWorkflow {
    Scan,
    Diff,
    Deep,
}

#[derive(Debug, Args)]
pub struct SecurityArgs {
    #[arg(long, value_enum, default_value = "scan")]
    pub workflow: SecurityWorkflow,
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ScientistWorkflow {
    Discover,
    Replicate,
}

#[derive(Debug, Args)]
pub struct ScientistArgs {
    #[arg(long, value_enum, default_value = "discover")]
    pub workflow: ScientistWorkflow,
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RsiWorkflow {
    Research,
    CreateEvals,
    BuildWorld,
    StressTest,
    Regression,
}

#[derive(Debug, Args)]
pub struct RsiArgs {
    #[arg(long, value_enum, default_value = "create-evals")]
    pub workflow: RsiWorkflow,
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

impl LoginArgs {
    pub fn method(&self) -> LoginMethod {
        if self.api_key {
            LoginMethod::ApiKey
        } else if self.device_code {
            LoginMethod::DeviceCode
        } else {
            self.method.unwrap_or(LoginMethod::Browser)
        }
    }
}

pub fn joined_prompt(parts: &[String]) -> Option<String> {
    let prompt = parts.join(" ");
    (!prompt.trim().is_empty()).then(|| prompt.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_is_a_mode_not_an_argv_secret() {
        let cli = Cli::try_parse_from(["clark", "login", "--api-key"]).unwrap();
        let Some(Command::Login(login)) = cli.command else {
            panic!("login expected")
        };
        assert_eq!(login.method(), LoginMethod::ApiKey);
        assert!(Cli::try_parse_from(["clark", "login", "--api-key", "ck_live_secret"]).is_err());
    }

    #[test]
    fn specialist_prompt_preserves_words() {
        let cli = Cli::try_parse_from(["clark", "scout", "map", "this", "repo"]).unwrap();
        let Some(Command::Scout(prompt)) = cli.command else {
            panic!("scout expected")
        };
        assert_eq!(
            joined_prompt(&prompt.prompt).as_deref(),
            Some("map this repo")
        );
    }

    #[test]
    fn headless_conversation_id_is_a_global_cloud_selector() {
        let cli = Cli::try_parse_from([
            "clark",
            "--conversation",
            "conversation-1",
            "scientist",
            "--workflow",
            "replicate",
            "continue",
        ])
        .unwrap();
        assert_eq!(cli.conversation.as_deref(), Some("conversation-1"));
        assert!(matches!(cli.command, Some(Command::Scientist(_))));
    }
}
