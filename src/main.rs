#![recursion_limit = "512"]

mod app;
mod components;
mod content;
mod emdash;
mod util;

#[tokio::main]
async fn main() {
    topcoat::start(app::router()).await.unwrap();
}
