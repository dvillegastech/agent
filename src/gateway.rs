use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

use crate::agent::runner::AgentRunner;
use crate::config::AgentConfig;
use crate::utils;

/// Shared state for the gateway.
struct GatewayState {
    runner: Mutex<AgentRunner>,
    model: String,
    provider: String,
}

/// Request body for the chat endpoint.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
}

/// Response body for the chat endpoint.
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub error: Option<String>,
}

/// Health check response.
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    model: String,
    provider: String,
}

/// Start the HTTP gateway server.
pub async fn run_gateway(config: AgentConfig, host: &str, port: u16) -> anyhow::Result<()> {
    let runner = AgentRunner::from_config(&config);

    let state = Arc::new(GatewayState {
        runner: Mutex::new(runner),
        model: config.model.clone(),
        provider: config.provider.to_string(),
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/chat", post(chat_handler))
        .route("/clear", post(clear_handler))
        .route("/stats", get(stats_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = format!("{host}:{port}");
    eprintln!(
        "\n{} HTTP Gateway listening on {}",
        "  [gateway]".bright_magenta(),
        bind_addr.bright_cyan()
    );
    eprintln!(
        "  {} POST /chat   - Send a message",
        "│".dimmed()
    );
    eprintln!(
        "  {} GET  /health - Health check",
        "│".dimmed()
    );
    eprintln!(
        "  {} POST /clear  - Clear conversation",
        "│".dimmed()
    );
    eprintln!(
        "  {} GET  /stats  - Conversation stats",
        "│".dimmed()
    );
    eprintln!(
        "  {} Model: {} ({})\n",
        "│".dimmed(),
        config.model.bright_white(),
        config.provider.to_string().cyan()
    );

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        model: state.model.clone(),
        provider: state.provider.clone(),
    })
}

async fn chat_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ChatRequest>,
) -> (StatusCode, Json<ChatResponse>) {
    if req.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ChatResponse {
                response: String::new(),
                error: Some("Message cannot be empty".into()),
            }),
        );
    }

    eprintln!(
        "{} Received: {}",
        "  [gateway]".bright_magenta(),
        utils::truncate(&req.message, 80).dimmed()
    );

    let mut runner = state.runner.lock().await;
    match runner.process_message(&req.message).await {
        Ok(response) => (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                error: None,
            }),
        ),
        Err(e) => {
            eprintln!("{} {}", "  [gateway error]".red(), e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatResponse {
                    response: String::new(),
                    error: Some(e.to_string()),
                }),
            )
        }
    }
}

async fn clear_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<ChatResponse> {
    let mut runner = state.runner.lock().await;
    runner.clear_conversation();
    eprintln!("{} Conversation cleared", "  [gateway]".bright_magenta());
    Json(ChatResponse {
        response: "Conversation cleared".into(),
        error: None,
    })
}

async fn stats_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<serde_json::Value> {
    let runner = state.runner.lock().await;
    Json(serde_json::json!({
        "stats": runner.stats(),
        "cost": runner.cost_summary(),
    }))
}
