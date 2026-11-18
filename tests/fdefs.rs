use parenox::{Session, Source};

const NS: &str = "(ns api (:require [clojure.spec.alpha :as s]))";

#[test]
fn fdef_checks_the_body_and_calls() {
    let source = format!(
        "{NS}
        (defn label [x] x)
        (s/fdef label :args (s/cat :x string?) :ret string?)
        (s/fdef bad-body :ret string? :args (s/cat))
        (defn bad-body [] 42)
        (defn bad-call {{:t/sig '[:-> string?]}} [] (label 42))"
    );
    let report = Session::default().check("api.clj", &source);
    assert!(report.complete, "{:#?}", report.diagnostics);
    assert_eq!(report.diagnostics.len(), 2);
    assert!(
        report.diagnostics.iter().all(|d| d.code == "PNX2322"
            && &source[d.span.start.offset..d.span.end.offset] == "42")
    );
}

#[test]
fn fdef_agrees_with_inline_variadic_contract() {
    let source = format!(
        "{NS}
        (defn send {{:t/sig '[string? & string? :-> string?]}} [x & more] x)
        (s/fdef api/send :args (s/cat :x string? :more (s/* string?)) :ret string?)
        (defn use-send {{:t/sig '[:-> string?]}} [] (send (send \"a\") \"b\" \"c\"))"
    );
    let report = Session::default().check("api.clj", &source);
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
    let invalid = source.replace("\"c\"", "42");
    let report = Session::default().check("api.clj", &invalid);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn bad_or_conflicting_fdefs_do_not_pass() {
    for (declarations, code) in [
        (
            "(defn f {:t/sig '[number? :-> number?]} [x] x)
             (s/fdef f :args (s/cat :x string?) :ret string?)",
            "PNX2790",
        ),
        (
            "(defn f [x] x) (s/fdef f :args (s/cat) :ret string?)",
            "PNX2554",
        ),
        (
            "(defn f [x] x) (s/fdef f :args (s/cat :x string?) :ret string? :fn identity)",
            "PNX9001",
        ),
        (
            "(defn f [x] x) (s/fdef f :args (s/cat :x (s/* string?) :y string?) :ret string?)",
            "PNX9001",
        ),
        (
            "(s/fdef f :args (s/cat :x string?) :ret string?)",
            "PNX9001",
        ),
        (
            "(defn f [x] x) (s/fdef f :args (s/cat :x string?) :ret string?)
             (s/fdef f :args (s/cat :x string?) :ret string?)",
            "PNX2790",
        ),
    ] {
        let report = Session::default().check("api.clj", &format!("{NS}\n{declarations}"));
        assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
        assert_eq!(report.diagnostics[0].code, code);
    }
}

#[test]
fn fdef_and_referenced_spec_edits_invalidate_callers() {
    let consumer = Source {
        path: "app.clj",
        source: "(ns app (:require [api :as api]))
         (defn f {:t/sig '[:-> string?]} [] (api/label \"ok\"))",
    };
    let api = format!(
        "{NS}
         (s/def ::input string?)
         (s/fdef label :args (s/cat :x ::input) :ret string?)
         (defn label [x] (str x))"
    );
    let check = |session: &mut Session, api: &str| {
        session.check_project(
            &[
                consumer,
                Source {
                    path: "api.clj",
                    source: api,
                },
            ],
            None,
        )
    };
    let mut session = Session::default();
    let first = check(&mut session, &api);
    assert!(first.ok && first.complete, "{:#?}", first.files);
    for edited in [
        api.replace(":x ::input", ":x number?"),
        api.replace("::input string?)", "::input number?)"),
    ] {
        let report = check(&mut session, &edited);
        assert_eq!(report.files[0].stats.checked_functions, 1);
        assert_eq!(report.files[0].diagnostics[0].code, "PNX2322");
        assert!(check(&mut session, &api).ok);
    }
}

#[test]
fn inline_multi_arity_signatures_can_use_a_quoted_list() {
    let report = Session::default().check(
        "api.clj",
        "(defn f {:t/sig '([:-> number?] [string? :-> string?])} ([] 0) ([x] x))
         (defn g {:t/sig '[:-> string?]} [] (f \"text\"))",
    );
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
}
