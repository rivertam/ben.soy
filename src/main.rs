#![recursion_limit = "512"]

mod app;
mod components;
mod content;
mod emdash;
mod server;
mod util;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match server::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
