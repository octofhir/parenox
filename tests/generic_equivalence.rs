use parenox::{Session, Source};

#[test]
fn swapping_binder_names_reuses_passing_local_and_cross_namespace_callers() {
    let provider = Source {
        path: "library.clj",
        source: "(ns library (:require [clojure.spec.alpha :as s]))
          (defn map-values
            {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :x a) :ret b)
                                  (s/coll-of a :kind vector?) :-> (s/coll-of b :kind vector?)])}
            [f xs] (mapv f xs))
          (defn preview {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
            (map-values #(str (inc %)) [1 2]))",
    };
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [library :as lib] [clojure.spec.alpha :as s]))
          (defn labels {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
            (lib/map-values #(str (inc %)) [1 2]))
          (defn missing {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
            (lib/map-values #(str %) []))",
    };
    let mut session = Session::default();
    let cold = session.check_project(&[provider, app], None);
    let [before] = cold.files[1].diagnostics.as_slice() else {
        panic!("{cold:#?}")
    };
    assert_eq!(before.code, "PNX9010");
    assert!(before.message.contains("parameter a "));

    let renamed = provider
        .source
        .replace("[a b]", "[b a]")
        .replace(":x a) :ret b", ":x b) :ret a")
        .replace(
            "coll-of a :kind vector?) :-> (s/coll-of b",
            "coll-of b :kind vector?) :-> (s/coll-of a",
        );
    let files = [
        Source {
            source: &renamed,
            ..provider
        },
        app,
    ];
    let warm = session.check_project(&files, None);
    assert!(warm.files[0].ok);
    assert_eq!(warm.files[0].stats.checked_functions, 1);
    assert_eq!(warm.files[0].stats.reused_functions, 1);
    assert_eq!(warm.files[1].stats.checked_functions, 1);
    assert_eq!(warm.files[1].stats.reused_functions, 1);
    assert!(
        warm.files[1].diagnostics[0]
            .message
            .contains("parameter b ")
    );

    let fresh = Session::default().check_project(&files, None);
    assert_eq!(
        serde_json::to_value(&warm.files[1].diagnostics).unwrap(),
        serde_json::to_value(&fresh.files[1].diagnostics).unwrap()
    );
}

#[test]
fn extern_binder_rename_is_not_a_type_change_but_moving_a_parameter_is() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
          (defn label {:t/sig '[:-> string?]} [] (v/pick \"text\" 42))",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/pick (t/all [a] [a number? :-> a])}",
    };
    let mut session = Session::default();
    assert!(session.check_project(&[app], Some(externs)).ok);

    let renamed = Source {
        source: "{vendor/pick (t/all [item] [item number? :-> item])}",
        ..externs
    };
    let report = session.check_project(&[app], Some(renamed));
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.files[0].stats.checked_functions, 0);
    assert_eq!(report.files[0].stats.reused_functions, 1);

    let changed = Source {
        source: "{vendor/pick (t/all [item] [string? item :-> item])}",
        ..externs
    };
    let report = session.check_project(&[app], Some(changed));
    assert_eq!(report.files[0].stats.checked_functions, 1);
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Number"));
}

#[test]
fn a_generic_callback_with_an_incompatible_bound_is_rejected() {
    let report = Session::default().check(
        "callbacks.clj",
        "
         (ns app (:require [clojure.spec.alpha :as s]))
         (defn same {:t/sig '(t/all [(a :< number?)] [a :-> a])} [x] x)
         (defn call-text
           {:t/sig '[(s/fspec :args (s/cat :x string?) :ret string?) :-> string?]}
           [f] (f \"text\"))
         (defn unchecked {:t/sig '[:-> string?]} [] (call-text same))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2322"),
        "{report:#?}"
    );
}
