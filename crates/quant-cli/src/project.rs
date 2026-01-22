//! Project context discovery and QUANT.md support
//!
//! Provides Claude Code-like project understanding by:
//! 1. Discovering QUANT.md project files
//! 2. Auto-detecting project type (Rust, Node, Python, etc.)
//! 3. Building a project structure summary
//! 4. Providing relevant context to the LLM

use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Project type detection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Go,
    Java,
    Unknown,
}

impl ProjectType {
    /// Get important files for this project type
    pub fn key_files(&self) -> &[&str] {
        match self {
            ProjectType::Rust => &["Cargo.toml", "Cargo.lock", "src/main.rs", "src/lib.rs"],
            ProjectType::Node => &["package.json", "package-lock.json", "tsconfig.json", "src/index.ts", "src/index.js"],
            ProjectType::Python => &["pyproject.toml", "setup.py", "requirements.txt", "main.py", "app.py"],
            ProjectType::Go => &["go.mod", "go.sum", "main.go"],
            ProjectType::Java => &["pom.xml", "build.gradle", "src/main/java"],
            ProjectType::Unknown => &[],
        }
    }

    /// Get ignore patterns for this project type
    pub fn ignore_patterns(&self) -> &[&str] {
        match self {
            ProjectType::Rust => &["target/", "*.rlib", "*.rmeta"],
            ProjectType::Node => &["node_modules/", "dist/", "build/", ".next/"],
            ProjectType::Python => &["__pycache__/", "*.pyc", ".venv/", "venv/", ".egg-info/"],
            ProjectType::Go => &["vendor/"],
            ProjectType::Java => &["target/", "build/", "*.class", "*.jar"],
            ProjectType::Unknown => &[],
        }
    }
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectType::Rust => write!(f, "Rust"),
            ProjectType::Node => write!(f, "Node.js"),
            ProjectType::Python => write!(f, "Python"),
            ProjectType::Go => write!(f, "Go"),
            ProjectType::Java => write!(f, "Java"),
            ProjectType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Parsed QUANT.md content
#[derive(Debug, Clone, Default)]
pub struct QuantFile {
    /// Raw content
    pub content: String,
    /// Project description (first paragraph or # header)
    pub description: Option<String>,
    /// Key instructions extracted
    pub instructions: Vec<String>,
    /// File path
    pub path: PathBuf,
}

impl QuantFile {
    /// Parse a QUANT.md file
    pub fn parse(path: PathBuf, content: String) -> Self {
        let mut description = None;
        let mut instructions = Vec::new();
        let mut in_instructions = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Extract description from first heading or paragraph
            if description.is_none() && !trimmed.is_empty() {
                if trimmed.starts_with("# ") {
                    description = Some(trimmed[2..].to_string());
                } else if !trimmed.starts_with('#') {
                    description = Some(trimmed.to_string());
                }
            }

            // Look for instructions section
            if trimmed.to_lowercase().contains("instruction") && trimmed.starts_with('#') {
                in_instructions = true;
                continue;
            }

            // Collect bullet points as instructions
            if in_instructions && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
                instructions.push(trimmed[2..].to_string());
            }

            // Reset on new heading
            if trimmed.starts_with("# ") && in_instructions {
                in_instructions = false;
            }
        }

        Self {
            content,
            description,
            instructions,
            path,
        }
    }
}

/// Project context containing all discovered information
#[derive(Debug, Clone)]
pub struct ProjectContext {
    /// Root directory of the project
    pub root: PathBuf,
    /// Detected project type
    pub project_type: ProjectType,
    /// QUANT.md content if found
    pub quant_file: Option<QuantFile>,
    /// Project name (from config file or directory)
    pub name: String,
    /// Key files that exist in the project
    pub key_files: Vec<PathBuf>,
    /// Directory structure summary
    pub structure: Vec<String>,
    /// Git information if available
    pub git_info: Option<GitInfo>,
}

/// Git repository information
#[derive(Debug, Clone)]
pub struct GitInfo {
    pub branch: String,
    pub has_uncommitted: bool,
    pub remote: Option<String>,
}

impl ProjectContext {
    /// Discover project context from a directory
    pub fn discover(start_dir: &Path) -> Option<Self> {
        let root = find_project_root(start_dir)?;
        info!(root = %root.display(), "Found project root");

        let project_type = detect_project_type(&root);
        debug!(project_type = %project_type, "Detected project type");

        let quant_file = find_quant_file(&root);
        if quant_file.is_some() {
            info!("Found QUANT.md");
        }

        let name = extract_project_name(&root, &project_type);
        let key_files = find_key_files(&root, &project_type);
        let structure = build_structure_summary(&root, &project_type);
        let git_info = get_git_info(&root);

        Some(Self {
            root,
            project_type,
            quant_file,
            name,
            key_files,
            structure,
            git_info,
        })
    }

    /// Generate a context string for the LLM system prompt
    pub fn to_system_context(&self) -> String {
        let mut ctx = String::new();

        ctx.push_str(&format!("# Project: {}\n", self.name));
        ctx.push_str(&format!("Type: {}\n", self.project_type));
        ctx.push_str(&format!("Root: {}\n\n", self.root.display()));

        // Add QUANT.md content if present
        if let Some(ref quant) = self.quant_file {
            ctx.push_str("## Project Instructions (from QUANT.md)\n\n");
            ctx.push_str(&quant.content);
            ctx.push_str("\n\n");
        }

        // Add git info
        if let Some(ref git) = self.git_info {
            ctx.push_str(&format!("## Git\n"));
            ctx.push_str(&format!("Branch: {}\n", git.branch));
            if git.has_uncommitted {
                ctx.push_str("Status: Has uncommitted changes\n");
            }
            if let Some(ref remote) = git.remote {
                ctx.push_str(&format!("Remote: {}\n", remote));
            }
            ctx.push_str("\n");
        }

        // Add structure summary
        if !self.structure.is_empty() {
            ctx.push_str("## Project Structure\n```\n");
            for line in &self.structure {
                ctx.push_str(line);
                ctx.push('\n');
            }
            ctx.push_str("```\n\n");
        }

        // Add key files
        if !self.key_files.is_empty() {
            ctx.push_str("## Key Files\n");
            for file in &self.key_files {
                if let Ok(rel) = file.strip_prefix(&self.root) {
                    ctx.push_str(&format!("- {}\n", rel.display()));
                }
            }
            ctx.push_str("\n");
        }

        ctx
    }
}

/// Find project root by looking for marker files
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let markers = [
        // Version control
        ".git",
        // Rust
        "Cargo.toml",
        // Node
        "package.json",
        // Python
        "pyproject.toml",
        "setup.py",
        // Go
        "go.mod",
        // Java
        "pom.xml",
        "build.gradle",
        // Generic
        "QUANT.md",
        ".quant",
    ];

    let mut current = start.to_path_buf();

    // Canonicalize to handle relative paths
    if let Ok(canonical) = current.canonicalize() {
        current = canonical;
    }

    loop {
        for marker in markers {
            if current.join(marker).exists() {
                return Some(current);
            }
        }

        if !current.pop() {
            break;
        }
    }

    // Fallback to start directory if no markers found
    Some(start.to_path_buf())
}

/// Detect project type from marker files
fn detect_project_type(root: &Path) -> ProjectType {
    if root.join("Cargo.toml").exists() {
        ProjectType::Rust
    } else if root.join("package.json").exists() {
        ProjectType::Node
    } else if root.join("pyproject.toml").exists() || root.join("setup.py").exists() {
        ProjectType::Python
    } else if root.join("go.mod").exists() {
        ProjectType::Go
    } else if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        ProjectType::Java
    } else {
        ProjectType::Unknown
    }
}

/// Find QUANT.md file in project root or parent directories
fn find_quant_file(root: &Path) -> Option<QuantFile> {
    let candidates = ["QUANT.md", "quant.md", ".quant/instructions.md"];

    for candidate in candidates {
        let path = root.join(candidate);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(QuantFile::parse(path, content));
            }
        }
    }

    None
}

/// Extract project name from config files or directory name
fn extract_project_name(root: &Path, project_type: &ProjectType) -> String {
    match project_type {
        ProjectType::Rust => {
            if let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) {
                if let Ok(parsed) = content.parse::<toml::Table>() {
                    if let Some(package) = parsed.get("package").and_then(|p| p.as_table()) {
                        if let Some(name) = package.get("name").and_then(|n| n.as_str()) {
                            return name.to_string();
                        }
                    }
                }
            }
        }
        ProjectType::Node => {
            if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(name) = parsed.get("name").and_then(|n| n.as_str()) {
                        return name.to_string();
                    }
                }
            }
        }
        _ => {}
    }

    // Fallback to directory name
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Find key files that exist in the project
fn find_key_files(root: &Path, project_type: &ProjectType) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // Check project-specific key files
    for key_file in project_type.key_files() {
        let path = root.join(key_file);
        if path.exists() {
            files.push(path);
        }
    }

    // Also check for common files
    let common = ["README.md", "README", "LICENSE", "CHANGELOG.md", "QUANT.md"];
    for file in common {
        let path = root.join(file);
        if path.exists() && !files.contains(&path) {
            files.push(path);
        }
    }

    files
}

/// Build a summary of the project structure
fn build_structure_summary(root: &Path, project_type: &ProjectType) -> Vec<String> {
    let mut structure = Vec::new();
    let ignore_patterns = project_type.ignore_patterns();

    // Get top-level directories and files
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut items: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                // Skip hidden files and ignored patterns
                !name.starts_with('.') &&
                !ignore_patterns.iter().any(|p| {
                    let pattern = p.trim_end_matches('/');
                    name == pattern || name.starts_with(pattern)
                })
            })
            .collect();

        items.sort_by_key(|e| e.file_name());

        for entry in items.iter().take(20) {  // Limit to avoid huge outputs
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

            if is_dir {
                structure.push(format!("{}/", name));
                // Add one level of subdirectories for important dirs
                if let Ok(sub_entries) = std::fs::read_dir(entry.path()) {
                    let mut sub_items: Vec<_> = sub_entries
                        .filter_map(|e| e.ok())
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .take(5)
                        .collect();
                    sub_items.sort_by_key(|e| e.file_name());

                    for sub in sub_items {
                        let sub_name = sub.file_name().to_string_lossy().to_string();
                        let sub_is_dir = sub.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        if sub_is_dir {
                            structure.push(format!("  {}/", sub_name));
                        } else {
                            structure.push(format!("  {}", sub_name));
                        }
                    }
                }
            } else {
                structure.push(name);
            }
        }
    }

    structure
}

/// Get git information if available
fn get_git_info(root: &Path) -> Option<GitInfo> {
    let git_dir = root.join(".git");
    if !git_dir.exists() {
        return None;
    }

    // Get current branch
    let head_path = git_dir.join("HEAD");
    let branch = std::fs::read_to_string(&head_path)
        .ok()
        .and_then(|content| {
            if content.starts_with("ref: refs/heads/") {
                Some(content.trim_start_matches("ref: refs/heads/").trim().to_string())
            } else {
                Some("detached".to_string())
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Check for uncommitted changes (simple check via index)
    let has_uncommitted = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    // Get remote URL
    let remote = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    Some(GitInfo {
        branch,
        has_uncommitted,
        remote,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_rust_project() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let project_type = detect_project_type(dir.path());
        assert_eq!(project_type, ProjectType::Rust);
    }

    #[test]
    fn test_detect_node_project() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();

        let project_type = detect_project_type(dir.path());
        assert_eq!(project_type, ProjectType::Node);
    }

    #[test]
    fn test_parse_quant_file() {
        let content = r#"# My Project

This is a test project.

## Instructions

- Always use async/await
- Follow Rust conventions
- Write tests for new code

## Notes

Some other notes here.
"#;

        let quant = QuantFile::parse(PathBuf::from("QUANT.md"), content.to_string());
        assert_eq!(quant.description, Some("My Project".to_string()));
        assert_eq!(quant.instructions.len(), 3);
        assert!(quant.instructions[0].contains("async/await"));
    }

    #[test]
    fn test_find_project_root() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("src").join("nested");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "").unwrap();

        let root = find_project_root(&sub);
        assert!(root.is_some());
        // Canonicalize both paths for comparison (handles macOS /private/var symlink)
        let expected = dir.path().canonicalize().unwrap();
        let actual = root.unwrap().canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_rust_project_name() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "my-cool-project"
version = "0.1.0"
"#,
        )
        .unwrap();

        let name = extract_project_name(dir.path(), &ProjectType::Rust);
        assert_eq!(name, "my-cool-project");
    }

    #[test]
    fn test_project_context_discover() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "test-project"
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("QUANT.md"),
            "# Test\n\n## Instructions\n- Be helpful",
        )
        .unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let ctx = ProjectContext::discover(dir.path());
        assert!(ctx.is_some());

        let ctx = ctx.unwrap();
        assert_eq!(ctx.name, "test-project");
        assert_eq!(ctx.project_type, ProjectType::Rust);
        assert!(ctx.quant_file.is_some());
    }
}
