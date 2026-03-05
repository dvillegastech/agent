use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Proveedor LLM soportado.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => write!(f, "anthropic"),
            ProviderKind::OpenAI => write!(f, "openai"),
        }
    }
}

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Configuración de seguridad del agente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Directorios permitidos para operaciones de archivo.
    pub allowed_dirs: Vec<PathBuf>,
    /// Comandos de shell bloqueados.
    pub blocked_commands: Vec<String>,
    /// Tamaño máximo de archivo en bytes (10 MB por defecto).
    pub max_file_size: u64,
    /// Timeout para comandos shell en segundos.
    pub command_timeout_secs: u64,
    /// Si se requiere confirmación del usuario para comandos shell.
    pub require_shell_confirmation: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_dirs: vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))],
            blocked_commands: vec![
                "rm -rf /".into(),
                "mkfs".into(),
                "dd if=/dev".into(),
                ":(){:|:&};:".into(),
                "chmod -R 777 /".into(),
                "shutdown".into(),
                "reboot".into(),
                "halt".into(),
                "init 0".into(),
                "init 6".into(),
            ],
            max_file_size: 10 * 1024 * 1024,
            command_timeout_secs: 30,
            require_shell_confirmation: true,
        }
    }
}

/// Configuración general del agente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub provider: ProviderKind,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub system_prompt: String,
    pub security: SecurityConfig,
    pub max_conversation_turns: usize,
    pub max_tool_iterations: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Anthropic,
            api_key: String::new(),
            model: DEFAULT_ANTHROPIC_MODEL.into(),
            base_url: None,
            max_tokens: 4096,
            temperature: 0.7,
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            security: SecurityConfig::default(),
            max_conversation_turns: 100,
            max_tool_iterations: 25,
        }
    }
}

impl AgentConfig {
    /// Carga configuración desde variables de entorno y valores por defecto.
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let provider = match std::env::var("RUSTCLAW_PROVIDER")
            .unwrap_or_else(|_| "anthropic".into())
            .to_lowercase()
            .as_str()
        {
            "openai" => ProviderKind::OpenAI,
            _ => ProviderKind::Anthropic,
        };

        let api_key_env = match provider {
            ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
            ProviderKind::OpenAI => "OPENAI_API_KEY",
        };

        let api_key = std::env::var(api_key_env).map_err(|_| {
            AgentError::Config(format!(
                "Missing {api_key_env}. Set it in your environment or .env file."
            ))
        })?;

        let model = std::env::var("RUSTCLAW_MODEL").unwrap_or_else(|_| match provider {
            ProviderKind::Anthropic => DEFAULT_ANTHROPIC_MODEL.into(),
            ProviderKind::OpenAI => DEFAULT_OPENAI_MODEL.into(),
        });

        let base_url = std::env::var("RUSTCLAW_BASE_URL").ok();

        let max_tokens = std::env::var("RUSTCLAW_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4096);

        let temperature = std::env::var("RUSTCLAW_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        Ok(Self {
            provider,
            api_key,
            model,
            base_url,
            max_tokens,
            temperature,
            ..Default::default()
        })
    }

    /// Retorna la URL base del proveedor.
    pub fn provider_url(&self) -> String {
        if let Some(ref url) = self.base_url {
            return url.clone();
        }
        match self.provider {
            ProviderKind::Anthropic => "https://api.anthropic.com".into(),
            ProviderKind::OpenAI => "https://api.openai.com".into(),
        }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are RustClaw, a powerful AI coding assistant running as a CLI agent.

You have access to the following tools:
- read_file: Read the contents of a file
- write_file: Write content to a file
- list_dir: List directory contents
- shell: Execute shell commands
- search_files: Search for patterns in files

Guidelines:
1. Always verify paths are within allowed directories before file operations.
2. Explain your reasoning before taking actions.
3. For destructive operations, confirm with the user first.
4. Write clean, idiomatic code when generating code.
5. Handle errors gracefully and report them clearly."#;
