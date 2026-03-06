use std::process::Command;

use colored::Colorize;

/// Check if we're inside a git repository.
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Auto-commit file changes made by the agent with a descriptive message.
/// Returns the commit hash if successful, or None if nothing to commit.
pub fn auto_commit(message: &str) -> Option<String> {
    if !is_git_repo() {
        return None;
    }

    // Stage all changes
    let add = Command::new("git")
        .args(["add", "-A"])
        .output()
        .ok()?;
    if !add.status.success() {
        return None;
    }

    // Check if there are staged changes
    let diff = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()
        .ok()?;
    if diff.status.success() {
        // No changes to commit
        return None;
    }

    // Commit with the message
    let commit_msg = format!("[rustclaw] {}", message);
    let output = Command::new("git")
        .args(["commit", "-m", &commit_msg])
        .output()
        .ok()?;

    if output.status.success() {
        // Extract short hash
        let hash = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "???".into());

        eprintln!(
            "{} {} {}",
            "  [git]".bright_magenta(),
            hash.bright_white(),
            commit_msg.dimmed()
        );
        Some(hash)
    } else {
        None
    }
}

/// Get the current branch name.
pub fn current_branch() -> Option<String> {
    Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// Get a short git status summary.
pub fn status_summary() -> String {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok();

    match output {
        Some(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = text.lines().collect();
            if lines.is_empty() {
                "clean".into()
            } else {
                format!("{} changed files", lines.len())
            }
        }
        _ => "not a git repo".into(),
    }
}
