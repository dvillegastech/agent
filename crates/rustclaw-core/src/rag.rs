use std::collections::HashMap;
use std::path::{Path, PathBuf};

use colored::Colorize;

/// Simple codebase indexer that builds a map of files and their signatures.
/// This is a lightweight RAG alternative: instead of embeddings, we build
/// a structural index (functions, structs, imports) that can be injected
/// into the system prompt for better context.
pub struct CodebaseIndex {
    /// Map of file path -> list of signatures (fn, struct, impl, etc.)
    pub entries: HashMap<PathBuf, Vec<String>>,
    pub total_files: usize,
    pub total_lines: usize,
}

/// Directories to skip during indexing.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", ".git", "__pycache__", ".venv",
    "venv", "dist", "build", ".next", "vendor",
];

/// File extensions to index.
const INDEX_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java",
    "c", "cpp", "h", "hpp", "rb", "ex", "exs", "zig",
    "toml", "yaml", "yml", "json",
];

impl CodebaseIndex {
    /// Build an index of the codebase starting from `root`.
    pub fn build(root: &Path) -> Self {
        let mut index = Self {
            entries: HashMap::new(),
            total_files: 0,
            total_lines: 0,
        };

        index.walk_dir(root, root, 0);
        index
    }

    /// Generate a compact summary for the system prompt.
    pub fn summary(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut summary = format!(
            "\n\n--- Project Index ({} files, ~{} lines) ---\n",
            self.total_files, self.total_lines
        );

        let mut sorted_entries: Vec<_> = self.entries.iter().collect();
        sorted_entries.sort_by_key(|(path, _)| (*path).clone());

        for (path, signatures) in sorted_entries {
            if signatures.is_empty() {
                summary.push_str(&format!("  {}\n", path.display()));
            } else {
                summary.push_str(&format!("  {} ({})\n", path.display(), signatures.join(", ")));
            }
        }

        summary
    }

    /// Print index stats to stderr.
    pub fn print_stats(&self) {
        if self.total_files > 0 {
            eprintln!(
                "{} {} files indexed, ~{} lines",
                "  [index]".bright_magenta(),
                self.total_files,
                self.total_lines
            );
        }
    }

    fn walk_dir(&mut self, path: &Path, root: &Path, depth: usize) {
        if depth > 15 {
            return;
        }

        let entries = match std::fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if entry_path.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    self.walk_dir(&entry_path, root, depth + 1);
                }
            } else if entry_path.is_file() {
                if let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) {
                    if INDEX_EXTENSIONS.contains(&ext) {
                        self.index_file(&entry_path, root);
                    }
                }
            }
        }
    }

    fn index_file(&mut self, path: &Path, root: &Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let relative = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        let lines = content.lines().count();
        self.total_lines += lines;
        self.total_files += 1;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let signatures = extract_signatures(&content, ext);

        self.entries.insert(relative, signatures);
    }
}

/// Extract top-level signatures from source code.
fn extract_signatures(content: &str, ext: &str) -> Vec<String> {
    let mut sigs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let sig = match ext {
            "rs" => {
                if trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub struct ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("pub enum ")
                    || trimmed.starts_with("enum ")
                    || trimmed.starts_with("pub trait ")
                    || trimmed.starts_with("impl ")
                    || trimmed.starts_with("pub mod ")
                    || trimmed.starts_with("mod ")
                {
                    // Extract just the name part
                    extract_name(trimmed)
                } else {
                    None
                }
            }
            "py" => {
                if trimmed.starts_with("def ") || trimmed.starts_with("class ") {
                    extract_name(trimmed)
                } else {
                    None
                }
            }
            "js" | "ts" | "jsx" | "tsx" => {
                if trimmed.starts_with("function ")
                    || trimmed.starts_with("export function ")
                    || trimmed.starts_with("export default function ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("export class ")
                    || trimmed.contains("const ") && trimmed.contains(" = (")
                {
                    extract_name(trimmed)
                } else {
                    None
                }
            }
            "go" => {
                if trimmed.starts_with("func ") || trimmed.starts_with("type ") {
                    extract_name(trimmed)
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(s) = sig {
            if !sigs.contains(&s) {
                sigs.push(s);
            }
        }
    }

    // Limit to most important signatures
    sigs.truncate(15);
    sigs
}

/// Extract the identifier name from a declaration line.
fn extract_name(line: &str) -> Option<String> {
    // Remove common prefixes
    let cleaned = line
        .trim_start_matches("pub ")
        .trim_start_matches("export ")
        .trim_start_matches("default ")
        .trim_start_matches("async ");

    // Get first word after keyword
    let parts: Vec<&str> = cleaned.splitn(3, |c: char| c.is_whitespace() || c == '(' || c == '<' || c == '{' || c == ':').collect();

    if parts.len() >= 2 {
        let name = parts[1].trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}
