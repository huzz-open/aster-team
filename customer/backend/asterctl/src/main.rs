#![deny(unsafe_code)]

use std::{path::PathBuf, process::ExitCode};

use clap::{Args, Parser, Subcommand};

mod claude;
mod codex;
mod user_environment;

mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}

#[derive(Debug, Parser)]
#[command(
    name = "asterctl",
    version,
    about = "Configure Aster Team integrations"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Configure an integration.
    Setup {
        #[command(subcommand)]
        target: SetupTarget,
    },
    /// Show local integration status without making network requests.
    Status {
        #[command(subcommand)]
        target: ReadTarget,
    },
    /// Diagnose local configuration, authentication, and connectivity.
    Doctor {
        #[command(subcommand)]
        target: ReadTarget,
    },
    /// Remove settings previously managed by asterctl.
    Remove {
        #[command(subcommand)]
        target: ReadTarget,
    },
    /// Show the asterctl version.
    Version,
}

#[derive(Debug, Subcommand)]
enum SetupTarget {
    /// Configure Codex Desktop to use Aster Team.
    Codex(CodexSetupArgs),
    /// Configure Claude CLI for one project directory.
    Claude(ClaudeSetupArgs),
}

#[derive(Debug, Subcommand)]
enum ReadTarget {
    Codex,
    Claude(ClaudeProjectArgs),
}

#[derive(Debug, Args)]
struct CodexSetupArgs {
    /// OpenAI-compatible API base URL ending in /v1.
    #[arg(long)]
    base_url: Option<String>,
    /// Prompt securely for a new Aster member API key.
    #[arg(long)]
    set_key: bool,
    /// Start Codex after setup completes successfully.
    #[arg(long)]
    launch: bool,
}

#[derive(Debug, Args)]
struct ClaudeSetupArgs {
    /// Anthropic-compatible API origin without /v1.
    #[arg(long)]
    base_url: String,
    /// Project directory that will contain .claude/settings.local.json. Defaults to the current directory.
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Prompt securely for an Aster member API key.
    #[arg(long)]
    set_key: bool,
    /// Start Claude in the configured project after setup completes successfully.
    #[arg(long)]
    launch: bool,
}

#[derive(Debug, Args)]
struct ClaudeProjectArgs {
    /// Project directory configured by asterctl. Defaults to the current directory.
    #[arg(long, default_value = ".")]
    project: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Setup {
            target: SetupTarget::Codex(arguments),
        } => {
            codex::setup(
                arguments.base_url.as_deref(),
                arguments.set_key,
                arguments.launch,
            )
            .await
        }
        Command::Setup {
            target: SetupTarget::Claude(arguments),
        } => {
            claude::setup(
                &arguments.base_url,
                &arguments.project,
                arguments.set_key,
                arguments.launch,
            )
            .await
        }
        Command::Status {
            target: ReadTarget::Codex,
        } => codex::status(),
        Command::Status {
            target: ReadTarget::Claude(arguments),
        } => claude::status(&arguments.project),
        Command::Doctor {
            target: ReadTarget::Codex,
        } => codex::doctor().await,
        Command::Doctor {
            target: ReadTarget::Claude(arguments),
        } => claude::doctor(&arguments.project).await,
        Command::Remove {
            target: ReadTarget::Codex,
        } => codex::remove(),
        Command::Remove {
            target: ReadTarget::Claude(arguments),
        } => claude::remove(&arguments.project),
        Command::Version => {
            println!("{}", version_banner());
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn version_banner() -> String {
    format!(
        "asterctl {}\nCommit:     {}",
        env!("CARGO_PKG_VERSION"),
        build_info::BUILD_COMMIT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_setup_defaults_to_the_current_directory() {
        let cli = Cli::try_parse_from([
            "asterctl",
            "setup",
            "claude",
            "--base-url",
            "https://aster.example.com",
            "--set-key",
        ])
        .expect("parse Claude setup");

        let Command::Setup {
            target: SetupTarget::Claude(arguments),
        } = cli.command
        else {
            panic!("expected Claude setup command");
        };
        assert_eq!(arguments.project, PathBuf::from("."));
    }

    #[test]
    fn claude_setup_accepts_a_relative_project_directory() {
        let cli = Cli::try_parse_from([
            "asterctl",
            "setup",
            "claude",
            "--base-url",
            "https://aster.example.com",
            "--project",
            "../my-project",
            "--set-key",
        ])
        .expect("parse Claude setup");

        let Command::Setup {
            target: SetupTarget::Claude(arguments),
        } = cli.command
        else {
            panic!("expected Claude setup command");
        };
        assert_eq!(arguments.project, PathBuf::from("../my-project"));
    }

    #[test]
    fn setup_launch_is_opt_in_for_both_clients() {
        let codex = Cli::try_parse_from([
            "asterctl",
            "setup",
            "codex",
            "--base-url",
            "https://aster.example.com/v1",
            "--set-key",
            "--launch",
        ])
        .expect("parse Codex setup with launch");
        let Command::Setup {
            target: SetupTarget::Codex(arguments),
        } = codex.command
        else {
            panic!("expected Codex setup command");
        };
        assert!(arguments.launch);

        let claude = Cli::try_parse_from([
            "asterctl",
            "setup",
            "claude",
            "--base-url",
            "https://aster.example.com",
            "--set-key",
            "--launch",
        ])
        .expect("parse Claude setup with launch");
        let Command::Setup {
            target: SetupTarget::Claude(arguments),
        } = claude.command
        else {
            panic!("expected Claude setup command");
        };
        assert!(arguments.launch);
    }

    #[test]
    fn version_banner_includes_the_short_build_commit() {
        let banner = version_banner();
        assert!(banner.starts_with(&format!("asterctl {}\n", env!("CARGO_PKG_VERSION"))));
        assert!(banner.contains(&format!("Commit:     {}", build_info::BUILD_COMMIT)));
        assert!(
            build_info::BUILD_COMMIT == "unknown"
                || ((7..=12).contains(&build_info::BUILD_COMMIT.len())
                    && build_info::BUILD_COMMIT
                        .bytes()
                        .all(|value| value.is_ascii_hexdigit()))
        );
    }
}
