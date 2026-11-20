use parenox_analysis::{Session, Source};

#[test]
fn resolves_prefixed_dependencies_aliases_and_referrals_as_full_namespaces() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app
  (:require (project.api (access :as a :refer [label]) [keys :as-alias k])
            [project.api access]
            (unused-prefix))
  (:use (project.api [formatting :only [wrap]])))
(defn text {:t/sig '[string? :-> string?]} [x]
  (let [_ [::k/category #::k{:category :news}]]
    (wrap (a/label (label x)))))"#,
    };
    let access = Source {
        path: "project/api/access.clj",
        source: "(ns project.api.access)\n(defn label {:t/sig '[string? :-> string?]} [x] x)",
    };
    let formatting = Source {
        path: "project/api/formatting.clj",
        source: "(ns project.api.formatting)\n(defn wrap {:t/sig '[string? :-> string?]} [x] x)",
    };
    let mut session = Session::default();
    let report = session.check_project(&[app, access, formatting], None);
    assert!(report.ok && report.complete, "{report:#?}");

    for (needle, name, provider) in [
        ("access :as", "access", access),
        ("(access :as a", "a", access),
        ("[label]", "label", access),
        ("a/label", "label", access),
        ("(label x)", "label", access),
        ("(wrap (", "wrap", formatting),
    ] {
        let offset = app.source.find(needle).unwrap() + needle.rfind(name).unwrap();
        let target = session.definition(app.path, offset).unwrap();
        assert_eq!(target.path, provider.path, "{needle}");
    }
    assert!(
        session
            .namespace_move(access.path, "project.api.access", "project.api.renamed")
            .unwrap_err()
            .contains("prefixed libspec")
    );
}

#[test]
fn prefixed_groups_reject_dotted_children_and_nested_groups() {
    for libspec in [
        "(project [api.access :as a])",
        "[project (api [access :as a])]",
    ] {
        let mut session = Session::default();
        let source = format!("(ns app (:require {libspec}))");
        let report = session.check("app.clj", &source);
        assert!(
            report.diagnostics.iter().any(|item| item.code == "PNX9001"),
            "{libspec}: {report:#?}"
        );
        let alias = source.find(":as a").unwrap() + ":as ".len();
        assert!(session.definition("app.clj", alias).is_none(), "{libspec}");
    }
}
