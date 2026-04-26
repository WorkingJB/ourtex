use orchext_server::{config::Config, router, AppState};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "orchext_server=info,axum=info,sqlx=warn".into()),
        )
        .compact()
        .init();

    let config = Config::from_env()?;
    tracing::info!(bind = %config.bind, "starting orchext-server");

    let db = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .connect(&config.database_url)
        .await?;

    orchext_server::migrate(&db).await?;
    tracing::info!("migrations applied");

    let state = AppState::new(db).with_secure_cookies(config.secure_cookies);
    let mut app = router(state);
    if let Some(cors) = orchext_server::cors_layer(&config.cors_allow_origins) {
        tracing::info!(
            origins = ?config.cors_allow_origins,
            "CORS enabled for explicit origins"
        );
        app = app.layer(cors);
    }

    // Outermost layer: every request is wrapped in a `request` span
    // carrying a fresh UUID. Any `tracing::info!`/`warn!`/etc. inside
    // a handler inherits the span's fields, so a single request's log
    // lines are greppable by `id=…` in Fly's log feed. The id is
    // generated server-side rather than honoured from `X-Request-Id`
    // so callers can't pollute or impersonate request IDs in our logs.
    app = app.layer(
        TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
            tracing::info_span!(
                "request",
                id = %uuid::Uuid::new_v4(),
                method = %req.method(),
                uri = %req.uri(),
            )
        }),
    );

    let addr: SocketAddr = config.bind.parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl+c handler");
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
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
