use super::*;
use std::time::{Instant, SystemTime};

const EXPORT: &str = include_str!("../../../tests/fixtures/clojuredocs.edn");

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path = env::temp_dir().join(format!("parenox-docs-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn cache(&self) -> Cache {
        Cache {
            dir: self.0.clone(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn background_fetch_publishes_a_complete_snapshot() {
    let fixture = Fixture::new("cold");
    let (start, started) = mpsc::channel();
    let (release, ready) = mpsc::channel();
    let docs = Docs::cached(fixture.cache(), false, move || {
        start.send(()).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(EXPORT.to_owned())
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(docs.snapshot().find("clojure.core/map").is_none());
    assert!(!fixture.cache().path().exists());

    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while docs.snapshot().find("clojure.core/map").is_none() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for ClojureDocs"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fs::read_to_string(fixture.cache().path()).unwrap(), EXPORT);
}

#[test]
fn failed_refresh_preserves_the_verified_snapshot() {
    let fixture = Fixture::new("stale");
    let cache = fixture.cache();
    let path = cache.path();
    fs::write(&path, EXPORT).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() - 2 * MAX_AGE)
        .unwrap();

    let docs = Docs::cached(fixture.cache(), true, || unreachable!("offline fetch"));
    for failure in [Err(io::Error::other("unavailable")), Ok("{}".into())] {
        assert!(cache.refresh(&docs.snapshot, || failure).is_err());
        assert!(docs.snapshot().find("clojure.core/map").is_some());
        assert_eq!(fs::read_to_string(&path).unwrap(), EXPORT);
    }
}
