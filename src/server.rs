//! Own the recovery loop for exactly the web server's lifetime. Recovery exits
//! use the same bounded request drain as ordinary SIGTERM/Ctrl+C shutdown.

use std::{future::Future, io, time::Duration};

use benjisponge::data::Data;
use tokio::net::TcpListener;
use topcoat::router::RouterService;

pub async fn run() -> io::Result<()> {
    let host = env_or("HOST", "127.0.0.1")?;
    let port: u16 = env_or("PORT", "3000")?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let listener = TcpListener::bind((host.as_str(), port)).await?;
    let data = Data::from_env();
    let service = RouterService::new(crate::app::router(data.clone()))
        .shutdown_timeout(Duration::from_secs(10));
    serve_until(
        listener,
        service,
        data.maintain_connection(),
        shutdown_signal(),
    )
    .await
}

async fn serve_until(
    listener: TcpListener,
    service: RouterService,
    recovery: impl Future<Output = ()>,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    let mut restart = false;
    topcoat::serve_until(listener, service, async {
        tokio::select! {
            () = shutdown => {}
            () = recovery => { restart = true; }
        }
    })
    .await?;
    if restart {
        Err(io::Error::other("database recovery requested a restart"))
    } else {
        Ok(())
    }
}

fn env_or(key: &str, fallback: &str) -> io::Result<String> {
    match std::env::var(key) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(fallback.into()),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{Notify, oneshot};
    use topcoat::{
        context::{Cx, app_context},
        router::{Body, Router, response::Response, route},
    };

    struct InFlight {
        started: Notify,
        finish: Notify,
    }

    #[route(GET "/drain-test")]
    async fn drain_test(cx: &Cx) -> topcoat::Result<Response> {
        let state = app_context::<Arc<InFlight>>(cx);
        state.started.notify_one();
        state.finish.notified().await;
        Ok(Response::new(Body::from("finished")))
    }

    #[tokio::test]
    async fn recovery_shutdown_drains_requests_then_reports_failure() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let state = Arc::new(InFlight {
                started: Notify::new(),
                finish: Notify::new(),
            });
            let router = Router::builder()
                .route(drain_test)
                .app_context(Arc::clone(&state))
                .build();
            let (stop, signal) = oneshot::channel();
            let server = tokio::spawn(serve_until(
                listener,
                RouterService::new(router),
                async {
                    signal.await.unwrap();
                },
                std::future::pending(),
            ));
            let request = tokio::spawn(async move {
                reqwest::get(format!("http://{address}/drain-test"))
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap()
            });
            state.started.notified().await;
            stop.send(()).unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(
                !server.is_finished(),
                "in-flight work must drain before exit"
            );
            state.finish.notify_one();
            assert_eq!(request.await.unwrap(), "finished");
            assert!(
                server
                    .await
                    .unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("restart")
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ordinary_shutdown_reports_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let service = RouterService::new(Router::builder().build());
        serve_until(listener, service, std::future::pending(), async {})
            .await
            .unwrap();
    }
}
