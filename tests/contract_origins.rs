use parenox::{ContractKind, Session, Source, output};
use serde_json::{json, to_value};

#[test]
fn return_and_builtin_diagnostics_show_the_contract_they_check() {
    let source = "
        (ns app)
        (defn inc-name {:t/sig '[:-> number?]} [] (inc \"bad\"))
        (defn label {:t/sig '[:-> string?]} [] 42)";
    let report = Session::default().check("app.clj", source);
    let [argument, result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(
        to_value(&argument.contract).unwrap(),
        json!({"name":"clojure.core/inc", "kind":"builtin"})
    );
    assert_eq!(
        to_value(&result.contract).unwrap(),
        json!({"name":"app/label", "kind":"t_sig", "path":"app.clj"})
    );
    for rendered in [
        output::pretty(&report, source, false),
        output::plain(&report),
    ] {
        assert!(rendered.contains("Contract for clojure.core/inc: built-in rule"));
        assert!(rendered.contains("Contract for app/label: :t/sig in app.clj"));
    }
}

#[test]
fn source_contract_wins_over_extern_and_keeps_its_origin_through_a_local_alias() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [provider :as p]))
            (defn label {:t/sig '[:-> string?]} [] (let [render p/render] (render 42)))",
    };
    let provider = Source {
        path: "provider.clj",
        source: "(ns provider (:require [clojure.spec.alpha :as s]))
            (s/fdef render :args (s/cat :text string?) :ret string?)
            (defn render [text] text)",
    };
    let externs = Source {
        path: "types/vendor.edn",
        source: "{provider/render [number? :-> string?]}",
    };
    let mut session = Session::default();
    let report = session.check_project(&[app, provider], Some(externs));
    assert_eq!(report.files[0].diagnostics.len(), 1, "{report:#?}");
    let origin = &report.files[0].diagnostics[0].contract;
    assert_eq!(
        to_value(origin).unwrap(),
        json!({"name":"provider/render", "kind":"s_fdef", "path":"provider.clj"})
    );
    let changed = Source {
        source: "(ns provider) (defn render {:t/sig '[string? :-> string?]} [text] text)",
        ..provider
    };
    let updated = session.check_project(&[app, changed], Some(externs));
    assert_eq!(updated.files[0].stats.checked_functions, 1);
    assert_eq!(
        updated.files[0].diagnostics[0]
            .contract
            .as_ref()
            .unwrap()
            .kind,
        ContractKind::TSig
    );
}

#[test]
fn moving_externs_refreshes_only_their_consumers_and_matches_a_clean_check() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v] [clojure.spec.alpha :as s]))
            (defn wrong-arity {:t/sig '[:-> string?]} [] (v/render))
            (defn via-apply {:t/sig '[(s/coll-of number? :kind vector?) :-> string?]} [xs]
              (apply v/join xs))
            (defn independent {:t/sig '[:-> number?]} [] 42)",
    };
    let externs = Source {
        path: "types/old.edn",
        source: "{vendor/render [string? :-> string?] vendor/join [& string? :-> string?]}",
    };
    let mut session = Session::default();
    let old = session.check_project(&[app], Some(externs));
    assert_eq!(old.files[0].diagnostics.len(), 2, "{old:#?}");
    assert_eq!(old.files[0].diagnostics[0].code, "PNX2554");
    for diagnostic in &old.files[0].diagnostics {
        let origin = diagnostic.contract.as_ref().unwrap();
        assert_eq!(origin.kind, ContractKind::Extern);
        assert_eq!(origin.path.as_deref(), Some(externs.path));
    }
    let warm = session.check_project(&[app], Some(externs));
    assert_eq!(warm.files[0].stats.reused_functions, 3);

    let moved = Source {
        path: "types/new.edn",
        ..externs
    };
    let edited = session.check_project(&[app], Some(moved));
    let clean = Session::default().check_project(&[app], Some(moved));
    assert_eq!(edited.files[0].stats.checked_functions, 2);
    assert_eq!(edited.files[0].stats.reused_functions, 1);
    assert_eq!(
        to_value(&edited.files[0].diagnostics).unwrap(),
        to_value(&clean.files[0].diagnostics).unwrap()
    );
    for diagnostic in &edited.files[0].diagnostics {
        assert_eq!(
            diagnostic.contract.as_ref().unwrap().path.as_deref(),
            Some(moved.path)
        );
    }
}

#[test]
fn externs_cannot_override_modeled_functions_or_macros() {
    let app = Source {
        path: "app.clj",
        source: "(ns app)
            (defn fail {:t/sig '[:-> number?]} [] (inc \"not a number\"))
            (defn lookup {:t/sig '[:-> number?]} [] (get {:id 1} :id))
            (defn thread {:t/sig '[:-> number?]} [] (-> 1 inc))
            (defn checked-name {:t/sig '[:-> #{\"abc\"}]} [] (name :demo/abc))
            (defn unmodeled {:t/sig '[:-> string?]} [] (namespace :demo/abc))",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{clojure.core/inc [string? :-> number?]
                  clojure.core/get [number? :-> string?]
                  clojure.core/-> [number? :-> string?]
                  clojure.string/upper-case [number? :-> number?]
                  clojure.core/name [keyword? :-> string?]
                  clojure.core/namespace [keyword? :-> string?]}",
    };
    let report = Session::default().check_project(&[app], Some(externs));
    assert_eq!(report.files[0].diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.files[0].diagnostics[0].code, "PNX2322");
    assert_eq!(report.files[1].diagnostics.len(), 5, "{report:#?}");
    for (diagnostic, name) in report.files[1].diagnostics.iter().zip([
        "clojure.core/inc",
        "clojure.core/get",
        "clojure.core/->",
        "clojure.string/upper-case",
        "clojure.core/name",
    ]) {
        assert_eq!(diagnostic.code, "PNX2790");
        assert_eq!(
            &externs.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            name
        );
        assert_eq!(
            to_value(&diagnostic.contract).unwrap(),
            json!({"name":name, "kind":"builtin"})
        );
    }
}
