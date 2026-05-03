/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use std::fmt;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anstyle::{AnsiColor, Style};
use clap::{Parser, Subcommand, ValueEnum};
use ophelia::service::{
    LocalServiceOptions, OPHELIA_MACH_SERVICE_NAME, OpheliaClient, OpheliaError,
    OpheliaInstallKind, OpheliaServiceInfo, TransferDestination, TransferRequest,
    TransferRequestSource,
};

#[derive(Debug, Parser)]
#[command(name = "ophelia")]
#[command(version, about = "Talk to a running Ophelia service")]
struct Cli {
    #[arg(
        long,
        value_enum,
        default_value_t = ColorChoice::Auto,
        global = true,
        help = "When to print colored output"
    )]
    color: ColorChoice,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Add a download to the running Ophelia app")]
    Add {
        #[arg(help = "HTTP or HTTPS URL to download")]
        url: String,
        #[arg(short, long, help = "Save to this file path")]
        output: Option<PathBuf>,
    },
    #[command(about = "Inspect the local Ophelia service")]
    Doctor,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    fn enabled(self, stream_is_terminal: bool) -> bool {
        match self {
            Self::Auto => stream_is_terminal,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

struct Theme {
    enabled: bool,
}

impl Theme {
    fn stdout(choice: ColorChoice) -> Self {
        Self {
            enabled: choice.enabled(io::stdout().is_terminal()),
        }
    }

    fn stderr(choice: ColorChoice) -> Self {
        Self {
            enabled: choice.enabled(io::stderr().is_terminal()),
        }
    }

    fn ok(&self, text: impl fmt::Display) -> String {
        self.paint(AnsiColor::Green.on_default().bold(), text)
    }

    fn error(&self, text: impl fmt::Display) -> String {
        self.paint(AnsiColor::Red.on_default().bold(), text)
    }

    fn label(&self, text: impl fmt::Display) -> String {
        self.paint(AnsiColor::Cyan.on_default().bold(), text)
    }

    fn muted(&self, text: impl fmt::Display) -> String {
        self.paint(AnsiColor::BrightBlack.on_default(), text)
    }

    fn paint(&self, style: Style, text: impl fmt::Display) -> String {
        if self.enabled {
            format!("{style}{text}{style:#}")
        } else {
            text.to_string()
        }
    }
}

pub async fn main_entry() -> ExitCode {
    let cli = Cli::parse();
    let stdout_theme = Theme::stdout(cli.color);
    let stderr_theme = Theme::stderr(cli.color);

    match run(cli.command, &stdout_theme).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&stderr_theme, error);
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command, theme: &Theme) -> Result<(), CliError> {
    match command {
        Command::Add { url, output } => add(url, output, theme).await,
        Command::Doctor => doctor(theme).await,
    }
}

async fn add(url: String, output: Option<PathBuf>, theme: &Theme) -> Result<(), CliError> {
    let client = OpheliaClient::connect_or_start_local(LocalServiceOptions::default())
        .map_err(CliError::from_service)?
        .client;
    let output = output.map(expand_tilde);
    let destination = match output {
        Some(ref path) => TransferDestination::ExplicitPath(path.clone()),
        None => TransferDestination::Automatic {
            suggested_filename: None,
        },
    };
    let request = TransferRequest {
        source: TransferRequestSource::Http { url: url.clone() },
        destination,
    };
    let id = client.add(request).await.map_err(CliError::from_service)?;

    println!("{}", theme.ok(format!("added transfer #{}", id.0)));
    print_field(theme, "source", &url);
    match &output {
        Some(path) => print_field(theme, "destination", path.display()),
        None => print_field(theme, "destination", "automatic"),
    }
    print_field(theme, "service", OPHELIA_MACH_SERVICE_NAME);

    Ok(())
}

async fn doctor(theme: &Theme) -> Result<(), CliError> {
    let client = OpheliaClient::connect_or_start_local(LocalServiceOptions::default())
        .map_err(CliError::from_service)?
        .client;
    let info = client
        .service_info()
        .await
        .map_err(CliError::from_service)?;
    let snapshot = client.snapshot().await.map_err(CliError::from_service)?;

    println!("{}", theme.ok("Ophelia service is reachable"));
    print_service_info(theme, &info);
    print_field(theme, "client", current_exe_display());
    print_field(theme, "transfers", snapshot.transfers.len());
    Ok(())
}

fn print_service_info(theme: &Theme, info: &OpheliaServiceInfo) {
    print_field(theme, "service", &info.service_name);
    print_field(theme, "version", &info.version);
    print_field(theme, "owner", install_kind_label(info.helper.install_kind));
    print_field(theme, "pid", info.helper.pid);
    print_field(
        theme,
        "binary",
        optional_path_display(info.helper.executable.as_deref()),
    );
    print_field(
        theme,
        "helper hash",
        info.helper
            .executable_sha256
            .as_deref()
            .unwrap_or("unavailable"),
    );
    print_field(theme, "endpoint", &info.endpoint.name);
    print_field(theme, "profile", info.profile.data_dir.display());
    print_field(theme, "settings", info.profile.settings_path.display());
    print_field(theme, "database", info.profile.database_path.display());
    print_field(theme, "logs", info.profile.logs_dir.display());
    print_field(
        theme,
        "downloads",
        info.profile.default_download_dir.display(),
    );
}

fn install_kind_label(kind: OpheliaInstallKind) -> &'static str {
    match kind {
        OpheliaInstallKind::AppBundle => "app bundle",
        OpheliaInstallKind::HomebrewFormula => "homebrew formula",
        OpheliaInstallKind::Development => "development build",
        OpheliaInstallKind::Other => "other",
        OpheliaInstallKind::Unknown => "unknown",
    }
}

fn print_field(theme: &Theme, label: &str, value: impl fmt::Display) {
    println!("{} {}", theme.label(format!("{label:<12}")), value);
}

fn current_exe_display() -> String {
    optional_path_display(std::env::current_exe().ok().as_deref())
}

fn optional_path_display(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn print_error(theme: &Theme, error: CliError) {
    eprintln!("{}", theme.error(error.heading()));
    eprintln!();
    eprintln!("{}", error.detail());
    if let Some(hint) = error.hint() {
        eprintln!("{}", theme.muted(hint));
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return home_dir().unwrap_or(path);
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    path
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Debug)]
enum CliError {
    ServiceClosed,
    Service(OpheliaError),
}

impl CliError {
    fn from_service(error: OpheliaError) -> Self {
        match error {
            OpheliaError::Closed => Self::ServiceClosed,
            error => Self::Service(error),
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Self::ServiceClosed => "could not reach Ophelia",
            Self::Service(_) => "Ophelia rejected the command",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::ServiceClosed => format!("Could not connect to {OPHELIA_MACH_SERVICE_NAME}"),
            Self::Service(error) => error.to_string(),
        }
    }

    fn hint(&self) -> Option<&'static str> {
        match self {
            Self::ServiceClosed => Some("Start the Ophelia service, then try again"),
            Self::Service(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_directory_prefix() {
        let Some(home) = home_dir() else {
            return;
        };

        assert_eq!(expand_tilde(PathBuf::from("~")), home);
        assert_eq!(
            expand_tilde(PathBuf::from("~/file.bin")),
            home.join("file.bin")
        );
    }

    #[test]
    fn leaves_non_home_paths_alone() {
        assert_eq!(
            expand_tilde(PathBuf::from("/tmp/file.bin")),
            PathBuf::from("/tmp/file.bin")
        );
        assert_eq!(
            expand_tilde(PathBuf::from("relative/file.bin")),
            PathBuf::from("relative/file.bin")
        );
    }
}
