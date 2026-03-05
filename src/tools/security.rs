use std::path::{Path, PathBuf};

use crate::config::SecurityConfig;
use crate::error::{AgentError, Result};

/// Pre-normalized dangerous command patterns (spaces already removed).
const DANGEROUS_PATTERNS: &[&str] = &[
    "curl|sh",
    "curl|bash",
    "wget|sh",
    "wget|bash",
    ">/dev/sd",
    "chmod777/",
    "chown-R",
    "passwd",
    "visudo",
];

/// Validador de seguridad para operaciones de herramientas.
pub struct SecurityGuard {
    config: SecurityConfig,
    /// Pre-canonicalized allowed directories for fast validation.
    canonical_allowed_dirs: Vec<PathBuf>,
}

impl SecurityGuard {
    pub fn new(config: SecurityConfig) -> Self {
        let canonical_allowed_dirs = config
            .allowed_dirs
            .iter()
            .filter_map(|d| d.canonicalize().ok())
            .collect();
        Self {
            config,
            canonical_allowed_dirs,
        }
    }

    /// Valida que una ruta de archivo esté dentro de los directorios permitidos.
    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let resolved = self.resolve_path(path)?;

        // Canonicalize directly; handle new files via parent
        let canonical = match resolved.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                let parent = resolved
                    .parent()
                    .ok_or_else(|| AgentError::Security("Invalid parent directory".into()))?;
                let canonical_parent = parent.canonicalize().map_err(|e| {
                    AgentError::Security(format!(
                        "Parent directory does not exist or cannot be resolved: {e}"
                    ))
                })?;
                let file_name = resolved
                    .file_name()
                    .ok_or_else(|| AgentError::Security("Invalid file name".into()))?;
                canonical_parent.join(file_name)
            }
        };

        // Check against pre-canonicalized allowed directories
        let allowed = self
            .canonical_allowed_dirs
            .iter()
            .any(|dir| canonical.starts_with(dir));

        if !allowed {
            return Err(AgentError::Security(format!(
                "Path '{}' is outside allowed directories. Allowed: {:?}",
                path, self.config.allowed_dirs
            )));
        }

        Ok(canonical)
    }

    /// Valida que un comando shell no esté bloqueado.
    pub fn validate_command(&self, command: &str) -> Result<()> {
        let trimmed = command.trim();

        for blocked in &self.config.blocked_commands {
            if trimmed.contains(blocked.as_str()) {
                return Err(AgentError::Security(format!(
                    "Command blocked: contains '{blocked}'"
                )));
            }
        }

        // Check against pre-normalized dangerous patterns
        let normalized = trimmed.replace(' ', "");
        for pattern in DANGEROUS_PATTERNS {
            if normalized.contains(pattern) {
                return Err(AgentError::Security(format!(
                    "Potentially dangerous command pattern detected: {pattern}"
                )));
            }
        }

        Ok(())
    }

    /// Valida tamaño de archivo.
    pub fn validate_file_size(&self, size: u64) -> Result<()> {
        if size > self.config.max_file_size {
            return Err(AgentError::Security(format!(
                "File size ({size} bytes) exceeds maximum allowed ({} bytes)",
                self.config.max_file_size
            )));
        }
        Ok(())
    }

    /// Returns the max file size for use in file filtering.
    pub fn max_file_size(&self) -> u64 {
        self.config.max_file_size
    }

    /// Timeout para comandos.
    pub fn command_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.config.command_timeout_secs)
    }

    fn resolve_path(&self, path: &str) -> Result<PathBuf> {
        let p = Path::new(path);
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            let cwd = std::env::current_dir()
                .map_err(|e| AgentError::Security(format!("Cannot get CWD: {e}")))?;
            Ok(cwd.join(p))
        }
    }
}
