//! Source directories in an explicitly resolved Clojure CLI classpath.
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub(super) fn directories(root: &Path, file: &Path) -> Result<Vec<String>, String> {
    let source =
        fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let mut seen = BTreeSet::new();
    let mut directories = Vec::new();
    for entry in env::split_paths(source.trim_end_matches(['\r', '\n'])) {
        if entry.as_os_str().is_empty() {
            continue;
        }
        let path = root.join(entry);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                let resolved = path
                    .canonicalize()
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                if seen.insert(resolved) {
                    directories.push(path.to_string_lossy().into_owned());
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    Ok(directories)
}
