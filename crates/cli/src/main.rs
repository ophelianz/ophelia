use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    ophelia_cli::main_entry().await
}
