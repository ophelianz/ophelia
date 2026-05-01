use std::fmt;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anstyle::{AnsiColor, Style};
use clap::{Parser, Subcommand, ValueEnum};
use ophelia::CorePaths;
use ophelia::session::{
    DownloadDestination, DownloadRequest, DownloadRequestSource, SessionClient, SessionDescriptor,
    SessionError, session_descriptor_path,
};

#[derive(Debug, Parser)]
#[command(name = "oph")]
#[command(version, about = "Talk to a running Ophelia session")]
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

#[tokio::main]
async fn main() -> ExitCode {
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
    }
}

async fn add(url: String, output: Option<PathBuf>, theme: &Theme) -> Result<(), CliError> {
    let paths = CorePaths::default_profile();
    let descriptor_path = session_descriptor_path(&paths);
    let descriptor = read_descriptor(&descriptor_path).await?;
    let client = SessionClient::connect_local(&descriptor).map_err(|error| {
        CliError::from_session(
            error,
            descriptor_path.clone(),
            descriptor.socket_path.clone(),
        )
    })?;
    let output = output.map(expand_tilde);
    let destination = match output {
        Some(ref path) => DownloadDestination::ExplicitPath(path.clone()),
        None => DownloadDestination::Automatic {
            suggested_filename: None,
        },
    };
    let request = DownloadRequest {
        source: DownloadRequestSource::Http { url: url.clone() },
        destination,
    };
    let id = client.add(request).await.map_err(|error| {
        CliError::from_session(
            error,
            descriptor_path.clone(),
            descriptor.socket_path.clone(),
        )
    })?;

    println!("{}", theme.ok(format!("added download #{}", id.0)));
    print_field(theme, "source", &url);
    match &output {
        Some(path) => print_field(theme, "destination", path.display()),
        None => print_field(theme, "destination", "automatic"),
    }
    print_field(theme, "session", descriptor.socket_path.display());

    Ok(())
}

async fn read_descriptor(path: &Path) -> Result<SessionDescriptor, CliError> {
    let body =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|source| CliError::SessionNotRunning {
                descriptor_path: path.to_path_buf(),
                source,
            })?;
    serde_json::from_str(&body).map_err(|source| CliError::BadDescriptor {
        descriptor_path: path.to_path_buf(),
        source,
    })
}

fn print_field(theme: &Theme, label: &str, value: impl fmt::Display) {
    println!("{} {}", theme.label(format!("{label:<12}")), value);
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
    SessionNotRunning {
        descriptor_path: PathBuf,
        source: io::Error,
    },
    BadDescriptor {
        descriptor_path: PathBuf,
        source: serde_json::Error,
    },
    SessionClosed {
        descriptor_path: PathBuf,
        socket_path: PathBuf,
    },
    Session(SessionError),
}

impl CliError {
    fn from_session(error: SessionError, descriptor_path: PathBuf, socket_path: PathBuf) -> Self {
        match error {
            SessionError::Closed => Self::SessionClosed {
                descriptor_path,
                socket_path,
            },
            error => Self::Session(error),
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Self::SessionNotRunning { .. } => "could not reach Ophelia",
            Self::BadDescriptor { .. } => "could not read Ophelia session",
            Self::SessionClosed { .. } => "could not reach Ophelia",
            Self::Session(_) => "Ophelia rejected the command",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::SessionNotRunning {
                descriptor_path,
                source,
            } => format!(
                "No running session was found at {} ({source})",
                descriptor_path.display()
            ),
            Self::BadDescriptor {
                descriptor_path,
                source,
            } => format!(
                "The session descriptor at {} is not valid JSON ({source})",
                descriptor_path.display()
            ),
            Self::SessionClosed {
                descriptor_path,
                socket_path,
            } => format!(
                "Found a session descriptor at {}, but the socket at {} is closed",
                descriptor_path.display(),
                socket_path.display()
            ),
            Self::Session(error) => error.to_string(),
        }
    }

    fn hint(&self) -> Option<&'static str> {
        match self {
            Self::SessionNotRunning { .. } => Some("Open Ophelia first, then try again"),
            Self::BadDescriptor { .. } => Some("Restart Ophelia so it writes a fresh session file"),
            Self::SessionClosed { .. } => Some("Open or restart Ophelia, then try again"),
            Self::Session(_) => None,
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
