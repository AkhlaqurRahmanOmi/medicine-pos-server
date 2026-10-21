use server::{create_router, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().init();

    let config = pos_core::Config::load_or_exit();

    let db = match db::Db::connect(&config).await {
        Ok(db) => {
            tracing::info!("Connected to database");
            Some(db)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to database: {e}");
            None
        }
    };

    let cache = match cache::Cache::from_config(&config, "medicine_pos") {
        Ok(cache) => {
            tracing::info!("Connected to cache");
            Some(cache)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to cache: {e}");
            None
        }
    };

    let port = config.server.port;
    let state = AppState::new(config, db, cache);
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
