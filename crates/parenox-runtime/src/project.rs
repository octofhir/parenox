use std::env;
use std::fs::{self, DirBuilder};
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use super::report::{Phase, RunError};

pub(super) fn project_root(project: &Path) -> Result<PathBuf, RunError> {
    let root = project.canonicalize().map_err(|error| {
        RunError::new(Phase::Project, format!("{}: {error}", project.display()))
    })?;
    if !root.is_dir() {
        return Err(RunError::new(
            Phase::Project,
            format!("{}: expected a project directory", root.display()),
        ));
    }
    Ok(root)
}

pub(super) fn execution_path(executable: &Path) -> Result<PathBuf, RunError> {
    if executable.is_relative() && executable.parent() != Some(Path::new("")) {
        let current =
            env::current_dir().map_err(|error| RunError::new(Phase::Launch, error.to_string()))?;
        Ok(current.join(executable))
    } else {
        Ok(executable.to_owned())
    }
}

pub(super) struct RunDirectory(pub PathBuf);

impl RunDirectory {
    pub fn create(name: &str) -> io::Result<Self> {
        let root = env::temp_dir().canonicalize()?;
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        for attempt in 0..16 {
            let path = root.join(format!(
                "parenox-{name}-{}-{started}-{attempt}",
                process::id()
            ));
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{name} run directory already exists"),
        ))
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
