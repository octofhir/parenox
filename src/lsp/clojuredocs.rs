//! Disk cache and background refresh for LSP documentation.
#[cfg(test)]
mod tests;

use std::env;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use parenox_lsp::clojuredocs::ClojureDocs;

const EXPORT_URL: &str = "https://raw.githubusercontent.com/clojure-emacs/clojuredocs-export-edn/master/exports/export.compact.edn";
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);

type Snapshot = Arc<RwLock<Arc<ClojureDocs>>>;

pub(super) struct Docs {
    snapshot: Snapshot,
    _stop_on_drop: Option<Sender<()>>,
}

impl Docs {
    pub(super) fn open(pinned: Option<&Path>, offline: bool) -> Result<Self, String> {
        if let Some(path) = pinned {
            let (docs, _) =
                read_snapshot(path).map_err(|error| format!("{}: {error}", path.display()))?;
            return Ok(Self::new(docs));
        }
        let Some(dir) = cache_directory() else {
            return Ok(Self::new(ClojureDocs::default()));
        };
        Ok(Self::cached(Cache { dir }, offline, download))
    }

    fn cached(
        cache: Cache,
        offline: bool,
        fetch: impl Fn() -> io::Result<String> + Send + 'static,
    ) -> Self {
        let (docs, delay) = cache.load().unwrap_or_default();
        let mut result = Self::new(docs);
        if !offline {
            let (stop, alive) = mpsc::channel();
            let snapshot = Arc::clone(&result.snapshot);
            if thread::Builder::new()
                .name("clojuredocs".into())
                .spawn(move || cache.refresh_loop(&snapshot, &alive, delay, fetch))
                .is_ok()
            {
                result._stop_on_drop = Some(stop);
            }
        }
        result
    }

    fn new(docs: ClojureDocs) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(Arc::new(docs))),
            _stop_on_drop: None,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<ClojureDocs> {
        Arc::clone(&self.snapshot.read().unwrap())
    }
}

struct Cache {
    dir: PathBuf,
}

impl Cache {
    fn path(&self) -> PathBuf {
        self.dir.join("clojuredocs.edn")
    }

    fn load(&self) -> Option<(ClojureDocs, Duration)> {
        let (docs, remaining) = read_snapshot(&self.path()).ok()?;
        (!docs.is_empty()).then_some((docs, remaining))
    }

    fn refresh_loop(
        &self,
        snapshot: &Snapshot,
        alive: &Receiver<()>,
        mut delay: Duration,
        fetch: impl Fn() -> io::Result<String>,
    ) {
        while alive.recv_timeout(delay) == Err(RecvTimeoutError::Timeout) {
            delay = self.refresh(snapshot, &fetch).unwrap_or(RETRY);
        }
    }

    fn refresh(
        &self,
        snapshot: &Snapshot,
        fetch: impl FnOnce() -> io::Result<String>,
    ) -> io::Result<Duration> {
        fs::create_dir_all(&self.dir)?;
        let lock = File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("clojuredocs.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(Duration::from_secs(1)),
            Err(TryLockError::Error(error)) => return Err(error),
        }

        // Another process may have filled the cache while we were waiting.
        if let Some((docs, remaining)) = self.load().filter(|(_, delay)| !delay.is_zero()) {
            *snapshot.write().unwrap() = Arc::new(docs);
            return Ok(remaining);
        }
        let source = fetch()?;
        let docs = ClojureDocs::parse(&source).map_err(io::Error::other)?;
        if docs.is_empty() {
            return Err(io::Error::other("empty ClojureDocs export"));
        }
        self.replace(&source)?;
        *snapshot.write().unwrap() = Arc::new(docs);
        Ok(MAX_AGE)
    }

    fn replace(&self, source: &str) -> io::Result<()> {
        let temp = self.dir.join("clojuredocs.download");
        match fs::remove_file(&temp) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut file = File::create_new(&temp)?;
        let written = file
            .write_all(source.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let replaced = written.and_then(|()| fs::rename(&temp, self.path()));
        if replaced.is_err() {
            let _ = fs::remove_file(&temp);
        }
        replaced
    }
}

fn cache_directory() -> Option<PathBuf> {
    let absolute = |name| {
        env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let base = absolute("XDG_CACHE_HOME").or_else(|| {
        if cfg!(windows) {
            absolute("LOCALAPPDATA")
        } else {
            let home = absolute("HOME")?;
            Some(home.join(if cfg!(target_os = "macos") {
                "Library/Caches"
            } else {
                ".cache"
            }))
        }
    })?;
    Some(base.join("parenox"))
}

fn read_snapshot(path: &Path) -> io::Result<(ClojureDocs, Duration)> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::other("expected a regular file"));
    }
    let file = File::open(path)?;
    let remaining = file
        .metadata()?
        .modified()
        .ok()
        .map_or(Duration::ZERO, |modified| {
            MAX_AGE.saturating_sub(modified.elapsed().unwrap_or_default())
        });
    let source = read_source(file)?;
    let docs = ClojureDocs::parse(&source).map_err(io::Error::other)?;
    Ok((docs, remaining))
}

fn read_source(input: impl Read) -> io::Result<String> {
    let mut source = String::new();
    input.take(MAX_BYTES + 1).read_to_string(&mut source)?;
    if source.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("ClojureDocs snapshot exceeds 16 MiB"));
    }
    Ok(source)
}

fn download() -> io::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(io::Error::other)?;
    let response = client
        .get(EXPORT_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(io::Error::other)?;
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "ClojureDocs HTTP {}",
            response.status()
        )));
    }
    read_source(response)
}
