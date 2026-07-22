use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Metadata {
    name: String,
    #[serde(rename = "repoUrl")]
    repo_url: Option<String>,
    #[serde(rename = "mainBranch")]
    main_branch: Option<String>,
}

#[derive(Serialize)]
struct NewWorkspaceMetadata<'a> {
    name: &'a str,
    #[serde(rename = "repoUrl")]
    repo_url: Option<String>,
    #[serde(rename = "mainBranch")]
    main_branch: &'a str,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Workspace {
    pub name: String,
    pub path: PathBuf,
    pub repo_url: Option<String>,
    pub main_branch: Option<String>,
    pub worktrees: Vec<Worktree>,
    pub projects: Vec<Project>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_base_repository: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Project {
    pub name: String,
    pub path: PathBuf,
}

pub(crate) fn default_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("DevTree"))
        .unwrap_or_else(|| PathBuf::from("DevTree"))
}

pub(crate) fn discover(root: &Path) -> io::Result<Vec<Workspace>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut workspaces = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let path = entry.path();
        let metadata_path = path.join(".devtree.json");
        let metadata = match fs::read_to_string(&metadata_path) {
            Ok(contents) => serde_json::from_str::<Metadata>(&contents).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid {}: {error}", metadata_path.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if metadata.name.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("workspace name is required in {}", metadata_path.display()),
            ));
        }

        let base_repository = path.join("repo-base");
        let worktrees = if base_repository.is_dir() {
            list_worktrees(&base_repository)?
        } else {
            Vec::new()
        };
        workspaces.push(Workspace {
            name: metadata.name,
            path: path.clone(),
            repo_url: metadata.repo_url,
            main_branch: metadata.main_branch,
            worktrees,
            projects: list_projects(&path.join("projects"))?,
        });
    }
    workspaces.sort_by_key(|workspace| workspace.name.to_lowercase());
    Ok(workspaces)
}

pub(crate) fn create_workspace(root: &Path, name: &str) -> io::Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace name must be a single directory name",
        ));
    }
    let path = root.join(name);
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("workspace already exists: {}", path.display()),
        ));
    }
    fs::create_dir_all(path.join("projects"))?;
    let metadata = NewWorkspaceMetadata {
        name,
        repo_url: None,
        main_branch: "main",
        created_at: format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ),
    };
    fs::write(
        path.join(".devtree.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;
    Ok(path)
}

fn list_projects(path: &Path) -> io::Result<Vec<Project>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut projects = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            projects.push(Project {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
            });
        }
    }
    projects.sort_by_key(|project| project.name.to_lowercase());
    Ok(projects)
}

fn list_worktrees(base_repository: &Path) -> io::Result<Vec<Worktree>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(base_repository)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git worktree list failed in {}: {}",
            base_repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut worktrees = Vec::new();
    let mut current_path = None;
    let mut current_branch = None;
    for line in String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(std::iter::once(""))
    {
        if line.is_empty() {
            if let Some(path) = current_path.take() {
                worktrees.push(Worktree {
                    is_base_repository: path == base_repository,
                    path,
                    branch: current_branch.take(),
                });
            }
        } else if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(PathBuf::from(path));
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_string());
        }
    }
    Ok(worktrees)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_metadata_projects_and_sorts_workspaces() {
        let root = std::env::temp_dir().join(format!("devtree-test-{}", std::process::id()));
        let alpha = root.join("alpha");
        let beta = root.join("beta");
        fs::create_dir_all(alpha.join("projects/app")).unwrap();
        fs::create_dir_all(&beta).unwrap();
        fs::write(
            alpha.join(".devtree.json"),
            r#"{"name":"Alpha","repoUrl":"https://example.com/alpha.git","mainBranch":"main"}"#,
        )
        .unwrap();
        fs::write(beta.join(".devtree.json"), r#"{"name":"beta"}"#).unwrap();

        let workspaces = discover(&root).unwrap();

        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].name, "Alpha");
        assert_eq!(workspaces[0].projects[0].name, "app");
        assert_eq!(workspaces[1].name, "beta");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_an_empty_workspace_with_metadata() {
        let root = std::env::temp_dir().join(format!("devtree-create-test-{}", std::process::id()));
        let path = create_workspace(&root, "new-project").unwrap();

        assert!(path.join("projects").is_dir());
        assert!(path.join(".devtree.json").is_file());
        assert_eq!(discover(&root).unwrap()[0].name, "new-project");
        fs::remove_dir_all(root).unwrap();
    }
}
