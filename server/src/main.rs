// GUI subsystem: the Plugin spawns us with `start "" /B`; this keeps a console window from appearing.
#![windows_subsystem = "windows"]

use aseprite_mcp_server::{Config, DEFAULT_PORT, Server};
use std::process::ExitCode;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() -> ExitCode {
    let port = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    // No console and no log yet: exit codes are the only signal for startup failures.
    let Some(token) = arg("--token") else { return ExitCode::from(2) };
    let mut config = Config::new(port, token);
    config.log = arg("--log").map(Into::into);

    match Server::start(config).await {
        Ok(server) => match server.wait().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(1),
        },
        Err(_) => ExitCode::from(2),
    }
}
