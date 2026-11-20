use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const LOADED_NAMESPACE: &str = "portable.nrepl-observed";
const SOURCE: &str = include_str!("fixtures/nrepl/namespace.cljc");
const JVM_SERVER: &str = r#"
(do
  (require '[nrepl.server :as server] '[nrepl.version :as version])
  (assert (= "1.5.2" (:version-string version/version)))
  (assert (= "1.13.0-alpha11" (clojure-version)))
  (let [listener (server/start-server :bind "127.0.0.1" :port 0)]
    (spit ".nrepl-port" (str (:port listener)))
    @(promise)))
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("parenox nrepl runtimes {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server {
    child: Child,
    root: PathBuf,
}

impl Server {
    fn start(mut command: Command, root: PathBuf) -> Self {
        fs::create_dir(&root).unwrap();
        let log = File::create(root.join("server.log")).unwrap();
        let child = command
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        Self { child, root }
    }

    fn port(&mut self) -> u16 {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = fs::read_to_string(self.root.join(".nrepl-port"))
                && let Ok(port) = text.trim().parse::<u16>()
                && port != 0
            {
                return port;
            }
            let status = self.child.try_wait().unwrap();
            assert!(
                status.is_none() && Instant::now() < deadline,
                "nREPL startup did not report a port ({status:?}): {}",
                fs::read_to_string(self.root.join("server.log")).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn supplied_path(variable: &str) -> PathBuf {
    let path = env::var_os(variable).unwrap_or_else(|| panic!("set {variable} to a local path"));
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|error| panic!("{variable}: {error}"))
}

fn jvm_classpath(nrepl: &Path) -> OsString {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("clojure")
        .current_dir(repository)
        .args(["-Srepro", "-Spath"])
        .output()
        .unwrap();
    assert!(output.status.success(), "clojure -Spath: {output:?}");
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let paths = env::split_paths(stdout.trim()).map(|path| repository.join(path));
    env::join_paths(std::iter::once(nrepl.to_owned()).chain(paths)).unwrap()
}

fn check_namespace_report(server: &mut Server, source: &Path, native: bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
    if native {
        command
            .env_clear()
            .env("PATH", "")
            .env("TMPDIR", &server.root);
    }
    let output = command
        .args(["nrepl", "--port", &server.port().to_string(), "--timeout-ms", "5000"])
        .arg("--load-file")
        .arg(source)
        .args([
            "--eval",
            "(do (println \"first eval λ\") [(str (ns-name *ns*)) portable.nrepl-observed/loaded-number])",
            "--eval",
            "(ns portable.nrepl-middle) (in-ns 'portable.nrepl-observed) (inc loaded-number)",
            "--eval",
            "(str (ns-name *ns*))",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["session_closed"], true);
    let [load] = report["loads"].as_array().unwrap().as_slice() else {
        panic!("unexpected load results: {report}");
    };
    assert_eq!(load["path"], source.to_str().unwrap());
    assert_eq!(load["out"], "loaded namespace λ\n");
    assert_eq!(load["status"], json!(["done"]));
    assert_eq!(load["done"], true);
    assert_eq!(
        load["values"].as_array().unwrap().last().unwrap(),
        ":loaded"
    );
    let first_namespace = if native {
        assert_eq!(load["namespace"], LOADED_NAMESPACE);
        LOADED_NAMESPACE
    } else {
        assert!(load.get("namespace").is_none(), "{load}");
        assert_eq!(load["values"], json!([":loaded"]));
        "user"
    };

    let [first, switched, next] = report["evals"].as_array().unwrap().as_slice() else {
        panic!("unexpected eval results: {report}");
    };
    assert_eq!(first["namespace"], first_namespace);
    assert_eq!(
        first["values"],
        json!([format!("[\"{first_namespace}\" 41]")])
    );
    assert_eq!(first["out"], "first eval λ\n");
    assert_eq!(switched["namespace"], LOADED_NAMESPACE);
    assert_eq!(switched["values"].as_array().unwrap().last().unwrap(), "42");
    assert_eq!(next["namespace"], LOADED_NAMESPACE);
    assert_eq!(next["values"], json!([format!("\"{LOADED_NAMESPACE}\"")]));
    for result in [first, switched, next] {
        assert_eq!(result["done"], true);
        assert_eq!(result["status"], json!(["done"]));
        assert_eq!(result["err"], "");
    }
}

#[test]
#[ignore = "requires local loopback, PARENOX_NREPL_JAVA, PARENOX_NREPL_JAR at 1.5.2 and PARENOX_CLJRS at 793ad97e"]
fn reports_last_namespace_from_real_jvm_and_native_nrepl() {
    let fixture = Fixture::new();
    let source = fixture.0.join("namespace.cljc");
    fs::write(&source, SOURCE).unwrap();

    let java = supplied_path("PARENOX_NREPL_JAVA");
    let jar = supplied_path("PARENOX_NREPL_JAR");
    let mut jvm = Command::new(java);
    jvm.arg("-cp")
        .arg(jvm_classpath(&jar))
        .args(["clojure.main", "-e", JVM_SERVER]);
    check_namespace_report(
        &mut Server::start(jvm, fixture.0.join("jvm")),
        &source,
        false,
    );

    let mut native = Command::new(supplied_path("PARENOX_CLJRS"));
    native.env_clear().env("PATH", "").args([
        "--no-config",
        "--no-native",
        "nrepl",
        "--bind",
        "127.0.0.1",
        "--port",
        "0",
    ]);
    check_namespace_report(
        &mut Server::start(native, fixture.0.join("native")),
        &source,
        true,
    );
}
