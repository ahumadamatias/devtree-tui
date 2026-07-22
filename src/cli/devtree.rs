use std::path::PathBuf;

pub(super) fn run_devtree_command(args: &[String]) -> std::io::Result<i32> {
    list_workspaces(args)
}

fn list_workspaces(args: &[String]) -> std::io::Result<i32> {
    let mut root = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --root");
                    return Ok(2);
                };
                root = Some(PathBuf::from(value));
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let root = root.unwrap_or_else(crate::devtree::default_root);
    let workspaces = crate::devtree::discover(&root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&workspaces)?);
        return Ok(0);
    }
    for workspace in workspaces {
        println!("{}\t{}", workspace.name, workspace.path.display());
        for worktree in workspace.worktrees {
            let label = worktree.branch.as_deref().unwrap_or("detached HEAD");
            println!("  worktree\t{label}\t{}", worktree.path.display());
        }
        for project in workspace.projects {
            println!("  project\t{}\t{}", project.name, project.path.display());
        }
    }
    Ok(0)
}
