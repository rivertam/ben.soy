//! Operational probes: never rendered through the site shell or cached.

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{Body, StatusCode, header, response::Response, route},
};

#[route(GET "/healthz")]
async fn live() -> Result<Response> {
    Ok(health_response(true))
}

#[route(GET "/readyz")]
async fn readiness_route(cx: &Cx) -> Result<Response> {
    Ok(health_response(
        app_context::<Data>(cx).readiness().await.is_ok(),
    ))
}

fn health_response(ready: bool) -> Response {
    Response::builder()
        .status(if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        })
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(if ready { "ok\n" } else { "not ready\n" }))
        .expect("static health response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::any;
    use topcoat::router::{Router, request::Request, to_bytes};

    async fn check(router: &Router, path: &str, status: StatusCode, body: &str) {
        let response = router
            .handle(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(to_bytes(response.into_body(), 128).await.unwrap(), body);
    }

    fn router(data: Data) -> Router {
        Router::builder()
            .route(live)
            .route(readiness_route)
            .layer(crate::app::response_layer::layer())
            .app_context(data)
            .build()
    }

    #[tokio::test]
    async fn probes_distinguish_liveness_from_database_readiness_without_bootstrapping() {
        let uninitialized = router(Data::new(Err("unused")));
        check(&uninitialized, "/healthz", StatusCode::OK, "ok\n").await;
        check(
            &uninitialized,
            "/readyz",
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready\n",
        )
        .await;

        let db = any::connect("mem://").await.unwrap();
        db.use_ns("health_test")
            .use_db("health_test")
            .await
            .unwrap();
        let data = Data::from_initialized_db(db);
        let db = data.db().await.unwrap();
        let initialized = router(data);
        check(
            &initialized,
            "/readyz",
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready\n",
        )
        .await;
        db.query("CREATE site_schema_migrations:baseline SET epoch = 1")
            .await
            .unwrap()
            .check()
            .unwrap();
        check(&initialized, "/readyz", StatusCode::OK, "ok\n").await;
        db.query("DELETE site_schema_migrations:baseline")
            .await
            .unwrap()
            .check()
            .unwrap();
        check(
            &initialized,
            "/readyz",
            StatusCode::SERVICE_UNAVAILABLE,
            "not ready\n",
        )
        .await;
        check(&initialized, "/healthz", StatusCode::OK, "ok\n").await;
    }
}
