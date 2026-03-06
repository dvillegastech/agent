use std::path::Path;

/// Load project-level context from RUSTCLAW.md if it exists.
/// This file acts like CLAUDE.md - persistent project instructions.
pub fn load_project_context() -> Option<String> {
    let candidates = [
        "RUSTCLAW.md",
        ".rustclaw.md",
        "rustclaw.md",
    ];

    for name in &candidates {
        let path = Path::new(name);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if !content.trim().is_empty() {
                    return Some(content);
                }
            }
        }
    }

    None
}

/// Load context from a specific directory (for --add-dir style usage).
#[allow(dead_code)]
pub fn load_context_from_dir(dir: &Path) -> Option<String> {
    let path = dir.join("RUSTCLAW.md");
    if path.exists() {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    }
}
