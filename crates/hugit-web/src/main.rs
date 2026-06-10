//! `hugit-web` — serve the githugr spine over the seeded fixture world.
//!
//! `HUGIT_WEB_ADDR` overrides the bind address (default `127.0.0.1:8790`).

use std::sync::Arc;

use hugit_web::fixture::FixtureProvider;

#[tokio::main]
async fn main() {
    let addr = std::env::var("HUGIT_WEB_ADDR").unwrap_or_else(|_| "127.0.0.1:8790".to_string());
    let provider = Arc::new(FixtureProvider::seed());
    let app = hugit_web::app(provider);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("hugit-web: cannot bind {addr}: {e}"));
    println!("hugit-web · servindo o spine em http://{addr}/ (mundo-fixture)");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("hugit-web: server error");
}
