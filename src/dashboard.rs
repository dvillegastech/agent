use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Sse,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::Local;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::CorsLayer;

use crate::agent::runner::AgentRunner;
use crate::config::AgentConfig;
use crate::session;

// ─── Shared Dashboard State ─────────────────────────────────────────

/// Metrics collected in real-time across all integrations.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardMetrics {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub active_sessions: Vec<ActiveSession>,
    pub recent_logs: Vec<LogEntry>,
    pub uptime_secs: u64,
}

impl Default for DashboardMetrics {
    fn default() -> Self {
        Self {
            total_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            active_sessions: Vec::new(),
            recent_logs: Vec::new(),
            uptime_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveSession {
    pub id: String,
    pub source: String, // "gateway", "telegram", "discord", "cli", "dashboard"
    pub messages: usize,
    pub last_activity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String, // "info", "warn", "error"
    pub source: String,
    pub message: String,
}

/// The shared state accessible from all dashboard routes.
pub struct DashboardState {
    runner: Mutex<AgentRunner>,
    config: AgentConfig,
    metrics: Mutex<DashboardMetrics>,
    start_time: std::time::Instant,
    events_tx: broadcast::Sender<String>,
}

impl DashboardState {
    fn new(runner: AgentRunner, config: AgentConfig) -> Self {
        let (events_tx, _) = broadcast::channel(64);
        Self {
            runner: Mutex::new(runner),
            config,
            metrics: Mutex::new(DashboardMetrics::default()),
            start_time: std::time::Instant::now(),
            events_tx,
        }
    }

    async fn add_log(&self, level: &str, source: &str, message: &str) {
        let entry = LogEntry {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            level: level.into(),
            source: source.into(),
            message: message.into(),
        };
        let mut metrics = self.metrics.lock().await;
        metrics.recent_logs.push(entry);
        if metrics.recent_logs.len() > 200 {
            metrics.recent_logs.remove(0);
        }
        // Notify SSE listeners
        let _ = self.events_tx.send("update".into());
    }

    async fn record_request(&self, input_tokens: u64, output_tokens: u64, cost: f64) {
        let mut metrics = self.metrics.lock().await;
        metrics.total_requests += 1;
        metrics.total_input_tokens += input_tokens;
        metrics.total_output_tokens += output_tokens;
        metrics.total_cost_usd += cost;
        let _ = self.events_tx.send("update".into());
    }
}

// ─── Launch Dashboard ───────────────────────────────────────────────

/// Start the web admin dashboard.
pub async fn run_dashboard(config: AgentConfig, host: &str, port: u16) -> anyhow::Result<()> {
    let runner = AgentRunner::from_config(&config);
    let state = Arc::new(DashboardState::new(runner, config.clone()));

    // Log startup
    state.add_log("info", "dashboard", "Dashboard started").await;

    let app = Router::new()
        // Pages
        .route("/", get(page_overview))
        .route("/sessions", get(page_sessions))
        .route("/config", get(page_config))
        .route("/logs", get(page_logs))
        .route("/chat", get(page_chat))
        // HTMX partials (polled for real-time updates)
        .route("/api/stats", get(api_stats))
        .route("/api/sessions-list", get(api_sessions_list))
        .route("/api/logs-list", get(api_logs_list))
        .route("/api/config-data", get(api_config_data))
        // Actions
        .route("/api/chat", post(api_chat))
        .route("/api/clear", post(api_clear))
        // SSE for real-time
        .route("/api/events", get(api_events))
        // Static assets
        .route("/assets/htmx.min.js", get(serve_htmx))
        .route("/assets/sse.js", get(serve_htmx_sse))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = format!("{host}:{port}");

    eprintln!(
        "\n{}",
        "  ┌──────────────────────────────────────────────┐"
            .bright_magenta()
    );
    eprintln!(
        "  {} {} {}",
        "│".bright_magenta(),
        "RustClaw Web Admin Dashboard".bright_white().bold(),
        "│".bright_magenta()
    );
    eprintln!(
        "{}",
        "  ├──────────────────────────────────────────────┤"
            .bright_magenta()
    );
    eprintln!(
        "  {} Dashboard:  http://{}",
        "│".bright_magenta(),
        bind_addr.bright_cyan()
    );
    eprintln!(
        "  {} Model:      {} ({})",
        "│".bright_magenta(),
        config.model.bright_white(),
        config.provider.to_string().cyan()
    );
    eprintln!(
        "{}",
        "  └──────────────────────────────────────────────┘\n"
            .bright_magenta()
    );

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ─── HTML Layout ────────────────────────────────────────────────────

fn layout(title: &str, active: &str, content: &str) -> Html<String> {
    Html(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title} - RustClaw Dashboard</title>
  <script src="/assets/htmx.min.js"></script>
  <script src="/assets/sse.js"></script>
  <style>
    * {{ margin: 0; padding: 0; box-sizing: border-box; }}
    :root {{
      --bg: #0f1117;
      --bg-card: #1a1d27;
      --bg-hover: #232736;
      --border: #2a2e3d;
      --text: #e4e6ef;
      --text-dim: #8b8fa3;
      --accent: #6c5ce7;
      --accent-light: #a29bfe;
      --green: #00b894;
      --orange: #fdcb6e;
      --red: #e17055;
      --blue: #74b9ff;
    }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      background: var(--bg);
      color: var(--text);
      min-height: 100vh;
      display: flex;
    }}
    /* Sidebar */
    .sidebar {{
      width: 240px;
      background: var(--bg-card);
      border-right: 1px solid var(--border);
      padding: 20px 0;
      position: fixed;
      height: 100vh;
      overflow-y: auto;
    }}
    .sidebar-logo {{
      padding: 0 20px 20px;
      border-bottom: 1px solid var(--border);
      margin-bottom: 12px;
    }}
    .sidebar-logo h1 {{
      font-size: 18px;
      color: var(--accent-light);
      font-weight: 700;
    }}
    .sidebar-logo small {{
      color: var(--text-dim);
      font-size: 11px;
    }}
    .nav-item {{
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 10px 20px;
      color: var(--text-dim);
      text-decoration: none;
      font-size: 14px;
      transition: all 0.15s;
    }}
    .nav-item:hover {{ background: var(--bg-hover); color: var(--text); }}
    .nav-item.active {{
      color: var(--accent-light);
      background: rgba(108,92,231,0.1);
      border-right: 3px solid var(--accent);
    }}
    .nav-icon {{ font-size: 18px; width: 24px; text-align: center; }}
    /* Main content */
    .main {{
      margin-left: 240px;
      flex: 1;
      padding: 28px 32px;
      min-height: 100vh;
    }}
    .page-header {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 24px;
    }}
    .page-header h2 {{ font-size: 22px; font-weight: 600; }}
    .badge {{
      display: inline-block;
      padding: 3px 10px;
      border-radius: 12px;
      font-size: 12px;
      font-weight: 500;
    }}
    .badge-green {{ background: rgba(0,184,148,0.15); color: var(--green); }}
    .badge-orange {{ background: rgba(253,203,110,0.15); color: var(--orange); }}
    .badge-blue {{ background: rgba(116,185,255,0.15); color: var(--blue); }}
    /* Cards grid */
    .stats-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
      gap: 16px;
      margin-bottom: 24px;
    }}
    .stat-card {{
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: 10px;
      padding: 20px;
    }}
    .stat-card .label {{
      font-size: 12px;
      color: var(--text-dim);
      text-transform: uppercase;
      letter-spacing: 0.5px;
      margin-bottom: 8px;
    }}
    .stat-card .value {{
      font-size: 28px;
      font-weight: 700;
    }}
    .stat-card .sub {{ font-size: 12px; color: var(--text-dim); margin-top: 4px; }}
    /* Table */
    .card {{
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: 10px;
      padding: 20px;
      margin-bottom: 20px;
    }}
    .card h3 {{
      font-size: 15px;
      font-weight: 600;
      margin-bottom: 16px;
      color: var(--text);
    }}
    table {{ width: 100%; border-collapse: collapse; }}
    th {{
      text-align: left;
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim);
      padding: 8px 12px;
      border-bottom: 1px solid var(--border);
    }}
    td {{
      padding: 10px 12px;
      font-size: 13px;
      border-bottom: 1px solid var(--border);
    }}
    tr:hover {{ background: var(--bg-hover); }}
    /* Log entries */
    .log-entry {{
      display: flex;
      gap: 12px;
      padding: 6px 0;
      font-size: 13px;
      font-family: 'SF Mono', 'Consolas', monospace;
      border-bottom: 1px solid rgba(42,46,61,0.5);
    }}
    .log-time {{ color: var(--text-dim); min-width: 70px; }}
    .log-level {{ min-width: 44px; font-weight: 600; }}
    .log-level.info {{ color: var(--blue); }}
    .log-level.warn {{ color: var(--orange); }}
    .log-level.error {{ color: var(--red); }}
    .log-src {{ color: var(--accent-light); min-width: 80px; }}
    .log-msg {{ color: var(--text); }}
    /* Chat */
    .chat-container {{
      display: flex;
      flex-direction: column;
      height: calc(100vh - 140px);
    }}
    .chat-messages {{
      flex: 1;
      overflow-y: auto;
      padding: 16px;
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: 10px 10px 0 0;
    }}
    .chat-msg {{
      margin-bottom: 16px;
      padding: 12px 16px;
      border-radius: 8px;
      max-width: 80%;
      font-size: 14px;
      line-height: 1.5;
      white-space: pre-wrap;
    }}
    .chat-msg.user {{
      background: var(--accent);
      color: white;
      margin-left: auto;
    }}
    .chat-msg.assistant {{
      background: var(--bg-hover);
      border: 1px solid var(--border);
    }}
    .chat-input-area {{
      display: flex;
      gap: 8px;
      padding: 12px;
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-top: none;
      border-radius: 0 0 10px 10px;
    }}
    .chat-input {{
      flex: 1;
      padding: 10px 14px;
      background: var(--bg);
      border: 1px solid var(--border);
      border-radius: 8px;
      color: var(--text);
      font-size: 14px;
      outline: none;
    }}
    .chat-input:focus {{ border-color: var(--accent); }}
    .btn {{
      padding: 10px 20px;
      background: var(--accent);
      color: white;
      border: none;
      border-radius: 8px;
      cursor: pointer;
      font-size: 14px;
      font-weight: 500;
      transition: background 0.15s;
    }}
    .btn:hover {{ background: var(--accent-light); }}
    .btn-sm {{ padding: 6px 14px; font-size: 12px; }}
    .btn-outline {{
      background: transparent;
      border: 1px solid var(--border);
      color: var(--text-dim);
    }}
    .btn-outline:hover {{ background: var(--bg-hover); color: var(--text); }}
    /* Config */
    .config-row {{
      display: flex;
      justify-content: space-between;
      padding: 12px 0;
      border-bottom: 1px solid var(--border);
      font-size: 14px;
    }}
    .config-key {{ color: var(--text-dim); }}
    .config-val {{ color: var(--text); font-weight: 500; }}
    /* Responsive */
    @media (max-width: 768px) {{
      .sidebar {{ width: 60px; }}
      .sidebar-logo h1, .sidebar-logo small, .nav-label {{ display: none; }}
      .main {{ margin-left: 60px; padding: 16px; }}
      .stats-grid {{ grid-template-columns: 1fr 1fr; }}
    }}
    /* Pulse animation for live indicator */
    .pulse {{
      display: inline-block;
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: var(--green);
      animation: pulse-anim 2s infinite;
    }}
    @keyframes pulse-anim {{
      0%, 100% {{ opacity: 1; }}
      50% {{ opacity: 0.4; }}
    }}
  </style>
</head>
<body>
  <nav class="sidebar">
    <div class="sidebar-logo">
      <h1>RustClaw</h1>
      <small>Admin Dashboard</small>
    </div>
    <a href="/" class="nav-item {ov_active}">
      <span class="nav-icon">&#9636;</span>
      <span class="nav-label">Overview</span>
    </a>
    <a href="/chat" class="nav-item {ch_active}">
      <span class="nav-icon">&#9993;</span>
      <span class="nav-label">Chat</span>
    </a>
    <a href="/sessions" class="nav-item {se_active}">
      <span class="nav-icon">&#9776;</span>
      <span class="nav-label">Sessions</span>
    </a>
    <a href="/config" class="nav-item {co_active}">
      <span class="nav-icon">&#9881;</span>
      <span class="nav-label">Config</span>
    </a>
    <a href="/logs" class="nav-item {lo_active}">
      <span class="nav-icon">&#9701;</span>
      <span class="nav-label">Logs</span>
    </a>
  </nav>
  <main class="main">
    {content}
  </main>
</body>
</html>"##,
        title = title,
        content = content,
        ov_active = if active == "overview" { "active" } else { "" },
        ch_active = if active == "chat" { "active" } else { "" },
        se_active = if active == "sessions" { "active" } else { "" },
        co_active = if active == "config" { "active" } else { "" },
        lo_active = if active == "logs" { "active" } else { "" },
    ))
}

// ─── Page Handlers ──────────────────────────────────────────────────

async fn page_overview(State(state): State<Arc<DashboardState>>) -> Html<String> {
    let config = &state.config;
    let content = format!(
        r##"
    <div class="page-header">
      <h2>Overview</h2>
      <div><span class="pulse"></span> <span style="color:var(--green);font-size:13px;margin-left:4px">Live</span></div>
    </div>

    <div id="stats-area" hx-get="/api/stats" hx-trigger="load, every 3s" hx-swap="innerHTML">
      <div class="stats-grid">
        <div class="stat-card"><div class="label">Loading...</div><div class="value">-</div></div>
      </div>
    </div>

    <div class="card">
      <h3>System Info</h3>
      <div class="config-row">
        <span class="config-key">Provider</span>
        <span class="config-val">{provider}</span>
      </div>
      <div class="config-row">
        <span class="config-key">Model</span>
        <span class="config-val">{model}</span>
      </div>
      <div class="config-row">
        <span class="config-key">Max Tokens</span>
        <span class="config-val">{max_tokens}</span>
      </div>
      <div class="config-row">
        <span class="config-key">Temperature</span>
        <span class="config-val">{temperature}</span>
      </div>
      <div class="config-row">
        <span class="config-key">Tool Iterations Limit</span>
        <span class="config-val">{max_tool_iter}</span>
      </div>
    </div>

    <div class="card">
      <h3>Recent Activity</h3>
      <div id="recent-logs" hx-get="/api/logs-list?limit=10" hx-trigger="load, every 5s" hx-swap="innerHTML">
        <p style="color:var(--text-dim);font-size:13px">Loading...</p>
      </div>
    </div>
"##,
        provider = config.provider,
        model = config.model,
        max_tokens = config.max_tokens,
        temperature = config.temperature,
        max_tool_iter = config.max_tool_iterations,
    );
    layout("Overview", "overview", &content)
}

async fn page_chat(_state: State<Arc<DashboardState>>) -> Html<String> {
    let content = r##"
    <div class="page-header">
      <h2>Chat</h2>
      <button class="btn btn-sm btn-outline" hx-post="/api/clear" hx-swap="none"
              onclick="document.getElementById('chat-msgs').innerHTML=''">Clear</button>
    </div>
    <div class="chat-container">
      <div class="chat-messages" id="chat-msgs"></div>
      <form class="chat-input-area" hx-post="/api/chat" hx-swap="none"
            hx-on::after-request="handleChatResponse(event)">
        <input type="text" name="message" class="chat-input" placeholder="Type a message..."
               autocomplete="off" id="chat-input">
        <button type="submit" class="btn">Send</button>
      </form>
    </div>
    <script>
      function handleChatResponse(event) {
        const input = document.getElementById('chat-input');
        const msgs = document.getElementById('chat-msgs');
        const userMsg = input.value;
        input.value = '';

        // Add user message
        const userDiv = document.createElement('div');
        userDiv.className = 'chat-msg user';
        userDiv.textContent = userMsg;
        msgs.appendChild(userDiv);

        // Parse response
        try {
          const data = JSON.parse(event.detail.xhr.responseText);
          const assistantDiv = document.createElement('div');
          assistantDiv.className = 'chat-msg assistant';
          assistantDiv.textContent = data.response || data.error || 'No response';
          msgs.appendChild(assistantDiv);
        } catch(e) {
          const errDiv = document.createElement('div');
          errDiv.className = 'chat-msg assistant';
          errDiv.textContent = 'Error processing response';
          msgs.appendChild(errDiv);
        }

        msgs.scrollTop = msgs.scrollHeight;
      }
    </script>
"##;
    layout("Chat", "chat", content)
}

async fn page_sessions(_state: State<Arc<DashboardState>>) -> Html<String> {
    let content = r##"
    <div class="page-header">
      <h2>Sessions</h2>
      <span class="badge badge-blue">Saved</span>
    </div>
    <div class="card">
      <h3>Saved Sessions</h3>
      <div id="sessions-table" hx-get="/api/sessions-list" hx-trigger="load, every 10s" hx-swap="innerHTML">
        <p style="color:var(--text-dim);font-size:13px">Loading...</p>
      </div>
    </div>
"##;
    layout("Sessions", "sessions", content)
}

async fn page_config(State(state): State<Arc<DashboardState>>) -> Html<String> {
    let content = r##"
    <div class="page-header">
      <h2>Configuration</h2>
      <span class="badge badge-green">Active</span>
    </div>
    <div class="card">
      <h3>Agent Configuration</h3>
      <div id="config-data" hx-get="/api/config-data" hx-trigger="load" hx-swap="innerHTML">
        <p style="color:var(--text-dim);font-size:13px">Loading...</p>
      </div>
    </div>
"##;
    let _ = state; // used for type checking
    layout("Config", "config", content)
}

async fn page_logs(_state: State<Arc<DashboardState>>) -> Html<String> {
    let content = r##"
    <div class="page-header">
      <h2>Logs</h2>
      <span class="badge badge-orange">Live</span>
    </div>
    <div class="card" style="max-height:calc(100vh - 140px);overflow-y:auto">
      <h3>Activity Log</h3>
      <div id="log-viewer" hx-get="/api/logs-list?limit=100" hx-trigger="load, every 3s" hx-swap="innerHTML">
        <p style="color:var(--text-dim);font-size:13px">Loading...</p>
      </div>
    </div>
"##;
    layout("Logs", "logs", content)
}

// ─── API Endpoints (HTMX partials) ─────────────────────────────────

async fn api_stats(State(state): State<Arc<DashboardState>>) -> Html<String> {
    let metrics = state.metrics.lock().await;
    let uptime = state.start_time.elapsed().as_secs();

    let uptime_str = if uptime >= 3600 {
        format!("{}h {}m", uptime / 3600, (uptime % 3600) / 60)
    } else if uptime >= 60 {
        format!("{}m {}s", uptime / 60, uptime % 60)
    } else {
        format!("{}s", uptime)
    };

    // Get runner stats
    let runner = state.runner.lock().await;
    let msg_count = runner.get_messages().len();
    drop(runner);

    let cost_str = if metrics.total_cost_usd >= 1.0 {
        format!("${:.2}", metrics.total_cost_usd)
    } else {
        format!("${:.4}", metrics.total_cost_usd)
    };

    Html(format!(
        r##"<div class="stats-grid">
  <div class="stat-card">
    <div class="label">Requests</div>
    <div class="value">{requests}</div>
    <div class="sub">total API calls</div>
  </div>
  <div class="stat-card">
    <div class="label">Input Tokens</div>
    <div class="value">{input_tokens}</div>
    <div class="sub">prompt tokens consumed</div>
  </div>
  <div class="stat-card">
    <div class="label">Output Tokens</div>
    <div class="value">{output_tokens}</div>
    <div class="sub">completion tokens generated</div>
  </div>
  <div class="stat-card">
    <div class="label">Estimated Cost</div>
    <div class="value" style="color:var(--green)">{cost}</div>
    <div class="sub">USD this session</div>
  </div>
  <div class="stat-card">
    <div class="label">Messages</div>
    <div class="value">{messages}</div>
    <div class="sub">in current conversation</div>
  </div>
  <div class="stat-card">
    <div class="label">Uptime</div>
    <div class="value">{uptime}</div>
    <div class="sub">since dashboard started</div>
  </div>
</div>"##,
        requests = metrics.total_requests,
        input_tokens = format_number(metrics.total_input_tokens),
        output_tokens = format_number(metrics.total_output_tokens),
        cost = cost_str,
        messages = msg_count,
        uptime = uptime_str,
    ))
}

async fn api_sessions_list(_state: State<Arc<DashboardState>>) -> Html<String> {
    let sessions = session::list_sessions(50).unwrap_or_default();

    if sessions.is_empty() {
        return Html(
            r#"<p style="color:var(--text-dim);font-size:13px;padding:12px 0">No saved sessions found.</p>"#
                .into(),
        );
    }

    let mut rows = String::new();
    for (id, updated, msg_count) in &sessions {
        rows.push_str(&format!(
            r#"<tr>
  <td><code style="color:var(--accent-light)">{id}</code></td>
  <td>{updated}</td>
  <td>{msg_count}</td>
</tr>"#,
        ));
    }

    Html(format!(
        r##"<table>
  <thead><tr><th>Session ID</th><th>Last Updated</th><th>Messages</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"##,
    ))
}

async fn api_logs_list(
    State(state): State<Arc<DashboardState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);

    let metrics = state.metrics.lock().await;
    let logs = &metrics.recent_logs;

    if logs.is_empty() {
        return Html(
            r#"<p style="color:var(--text-dim);font-size:13px;padding:8px 0">No log entries yet.</p>"#
                .into(),
        );
    }

    let start = if logs.len() > limit {
        logs.len() - limit
    } else {
        0
    };

    let mut html = String::new();
    for entry in logs[start..].iter().rev() {
        html.push_str(&format!(
            r#"<div class="log-entry">
  <span class="log-time">{}</span>
  <span class="log-level {}">{}</span>
  <span class="log-src">{}</span>
  <span class="log-msg">{}</span>
</div>"#,
            entry.timestamp,
            entry.level,
            entry.level.to_uppercase(),
            entry.source,
            html_escape(&entry.message),
        ));
    }

    Html(html)
}

async fn api_config_data(State(state): State<Arc<DashboardState>>) -> Html<String> {
    let c = &state.config;
    Html(format!(
        r##"
<div class="config-row"><span class="config-key">Provider</span><span class="config-val">{provider}</span></div>
<div class="config-row"><span class="config-key">Model</span><span class="config-val">{model}</span></div>
<div class="config-row"><span class="config-key">Base URL</span><span class="config-val">{base_url}</span></div>
<div class="config-row"><span class="config-key">Max Tokens</span><span class="config-val">{max_tokens}</span></div>
<div class="config-row"><span class="config-key">Temperature</span><span class="config-val">{temperature}</span></div>
<div class="config-row"><span class="config-key">Max Conversation Turns</span><span class="config-val">{max_turns}</span></div>
<div class="config-row"><span class="config-key">Max Tool Iterations</span><span class="config-val">{max_tool}</span></div>
<div class="config-row"><span class="config-key">Command Timeout</span><span class="config-val">{timeout}s</span></div>
<div class="config-row"><span class="config-key">Shell Confirmation</span><span class="config-val">{shell_confirm}</span></div>
<div class="config-row"><span class="config-key">Sandbox Mode</span><span class="config-val">{sandbox}</span></div>
<div class="config-row"><span class="config-key">Git Auto-Commit</span><span class="config-val">{git_auto}</span></div>
<div class="config-row"><span class="config-key">Allowed Dirs</span><span class="config-val">{allowed_dirs}</span></div>
<div class="config-row"><span class="config-key">Blocked Commands</span><span class="config-val">{blocked_count} patterns</span></div>
"##,
        provider = c.provider,
        model = c.model,
        base_url = c.provider_url(),
        max_tokens = c.max_tokens,
        temperature = c.temperature,
        max_turns = c.max_conversation_turns,
        max_tool = c.max_tool_iterations,
        timeout = c.security.command_timeout_secs,
        shell_confirm = c.security.require_shell_confirmation,
        sandbox = c.security.sandbox_mode,
        git_auto = c.security.git_auto_commit,
        allowed_dirs = c
            .security
            .allowed_dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        blocked_count = c.security.blocked_commands.len(),
    ))
}

// ─── Chat API ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChatForm {
    message: String,
}

async fn api_chat(
    State(state): State<Arc<DashboardState>>,
    axum::extract::Form(form): axum::extract::Form<ChatForm>,
) -> Json<serde_json::Value> {
    let message = form.message.trim().to_string();
    if message.is_empty() {
        return Json(serde_json::json!({"error": "Empty message"}));
    }

    state
        .add_log("info", "dashboard", &format!("User: {}", truncate_str(&message, 100)))
        .await;

    let mut runner = state.runner.lock().await;
    match runner.process_message(&message).await {
        Ok(response) => {
            // Record approximate metrics
            state.record_request(0, 0, 0.0).await;
            state
                .add_log("info", "dashboard", &format!("Assistant: {}", truncate_str(&response, 100)))
                .await;
            Json(serde_json::json!({"response": response}))
        }
        Err(e) => {
            state
                .add_log("error", "dashboard", &format!("Error: {e}"))
                .await;
            Json(serde_json::json!({"error": e.to_string()}))
        }
    }
}

async fn api_clear(State(state): State<Arc<DashboardState>>) -> Json<serde_json::Value> {
    let mut runner = state.runner.lock().await;
    runner.clear_conversation();
    state
        .add_log("info", "dashboard", "Conversation cleared")
        .await;
    Json(serde_json::json!({"status": "cleared"}))
}

// ─── SSE for real-time updates ──────────────────────────────────────

async fn api_events(
    State(state): State<Arc<DashboardState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok().map(|data| Ok(Event::default().data(data)))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ─── Embedded HTMX (minified stub - loads from CDN fallback) ────────

async fn serve_htmx() -> impl IntoResponse {
    // Serve a small loader that fetches HTMX from CDN
    let js = r#"/* HTMX loader - fetches from CDN */
(function(){
  var s = document.createElement('script');
  s.src = 'https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js';
  s.onload = function(){ htmx.process(document.body); };
  document.head.appendChild(s);
})();"#;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}

async fn serve_htmx_sse() -> impl IntoResponse {
    let js = r#"/* HTMX SSE extension loader */
(function(){
  var s = document.createElement('script');
  s.src = 'https://unpkg.com/htmx-ext-sse@2.2.2/sse.js';
  document.head.appendChild(s);
})();"#;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}

// ─── Utilities ──────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
