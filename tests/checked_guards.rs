use parenox::{Session, Source};

const EXAMPLE: &str = include_str!("../examples/checked_guards.clj");

#[test]
fn non_nil_guards_track_let_results_without_refining_shadowed_arguments() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [vendor.find :as f] [clojure.spec.alpha :as s]))
(defn prefix {:t/sig '[(s/nilable string?) :-> string?]} [input]
  (let [index (f/search input) saved index]
    (if (nil? saved) "" (subs input 0 saved))))
(defn bound {:t/sig '[(s/nilable string?) :-> string?]} [input]
  (if-some [index (f/search input 1)] (subs input index) ""))
(defn present-false {:t/sig '[(s/nilable string?) :-> string?]} [input]
  (if-some [_ (f/flag input)] (subs input 0) ""))"#,
    };
    let externs = Source {
        path: "find.edn",
        source: "{vendor.find/search {:sig [[t/any :-> (clojure.spec.alpha/nilable int?)]
                                           [t/any int? :-> (clojure.spec.alpha/nilable int?)]]
                                      :guard [:some 1 string?]}
                  vendor.find/flag {:sig [t/any :-> (clojure.spec.alpha/nilable #{false})] :guard [:some 1 string?]}}",
    };
    let mut session = Session::default();
    let report = session.check_project(&[app], Some(externs));
    assert!(report.ok && report.complete, "{report:#?}");

    let source = app.source.replace("saved index]", "saved index input nil]");
    let report = session.check_project(
        &[Source {
            source: &source,
            ..app
        }],
        Some(externs),
    );
    let [error] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.span.start.line, 4);
}

#[test]
fn extern_guard_argument_must_exist_at_every_arity() {
    let externs = Source {
        path: "find.edn",
        source: "{vendor.find/search {:sig [[t/any :-> boolean?] [t/any t/any :-> boolean?]]
                                      :guard [:then 2 string?]}}",
    };
    let report = Session::default().check_project(&[], Some(externs));
    assert_eq!(
        report.files[0].diagnostics[0].code, "PNX9008",
        "{report:#?}"
    );
}

#[test]
fn extern_guards_narrow_the_selected_argument_and_follow_snapshot_edits() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [example.text :as text] [clojure.spec.alpha :as s]))
(defn prefix {:t/sig '[(s/nilable string?) :-> string?]} [value]
  (if (text/has-prefix? "x" value) (subs value 1) "fallback"))"#,
    };
    let guarded = Source {
        path: "text.edn",
        source: "{example.text/has-prefix? {:guard [:then 2 string?]
                         :sig [string? t/any :-> boolean?]}}",
    };
    let mut session = Session::default();
    let report = session.check_project(&[app], Some(guarded));
    assert!(report.ok && report.complete, "{report:#?}");
    let plain = Source {
        source: "{example.text/has-prefix? [string? t/any :-> boolean?]}",
        ..guarded
    };
    let report = session.check_project(&[app], Some(plain));
    assert!(
        report.files[0]
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322"),
        "{report:#?}"
    );
    let report = session.check_project(&[app], Some(guarded));
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong_index = guarded.source.replace(":then 2", ":then 3");
    let report = session.check_project(
        &[app],
        Some(Source {
            source: &wrong_index,
            ..guarded
        }),
    );
    assert!(
        report.files[1]
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9008"),
        "{report:#?}"
    );
}

#[test]
fn checked_guards_refine_true_and_proven_false_branches() {
    let source = format!(
        "{EXAMPLE}
         (defn nilish?
           {{:t/sig '[(s/nilable string?) :-> boolean?] :t/guard '[:both x nil?]}}
           [x] (not (or (some? x) false)))
         (defn missing-value {{:t/sig '[(s/nilable string?) :-> nil?]}} [x]
           (if (nilish? x) x nil))",
    );
    let report = Session::default().check("guards.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn explicit_quotes_preserve_contracts_guards_and_named_spec_ranges() {
    let path = "quoted-contract.clj";
    let source = r#"
(ns reader.explicit-contract-quote
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))
(s/def ::text string?)

(defn ^{:t/sig (quote [(s/nilable ::text) :-> boolean?])
        :t/guard (quote [:both value string?])}
  text? [value] (string? value))

(defn trimmed {:t/sig (quote [(s/nilable ::text) :-> string?])} [value]
  (if (text? value) (str/trim value) "absent"))
"#;
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");
    let use_site = source.rfind("::text").unwrap();
    let definition = session.definition(path, use_site).unwrap();
    assert_eq!(definition.span.start.offset, source.find("::text").unwrap());
    assert_eq!(
        &source[definition.span.start.offset..definition.span.end.offset],
        "::text"
    );

    let qualified = source.replace("(quote [:both", "(clojure.core/quote [:both");
    let rejected = session.check(path, &qualified);
    assert!(!rejected.complete, "{rejected:#?}");
    assert!(rejected.diagnostics.iter().any(|finding| {
        finding.code == "PNX9008" && finding.message.contains("must be quoted")
    }));
}

#[test]
fn a_one_sided_guard_does_not_refine_its_false_branch() {
    let source = format!(
        "{EXAMPLE}
         (defn bad {{:t/sig '[boolean? (s/nilable string?) :-> nil?]}} [enabled value]
           (if (enabled-text? enabled value) nil value))"
    );
    let report = Session::default().check("guards.clj", &source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "PNX2322");
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}

#[test]
fn rejecting_an_implication_never_publishes_the_guard() {
    for (source, part) in [
        (
            EXAMPLE.replace("(if (string? value) true false)", "true"),
            "true result",
        ),
        (
            EXAMPLE.replace("[:then value string?]", "[:both value string?]"),
            "false result",
        ),
    ] {
        let report = Session::default().check("guards.clj", &source);
        assert!(!report.ok && !report.complete, "{report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "PNX9008" && d.message.contains(part)),
            "{report:#?}"
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX2322"),
            "the caller cannot use an unproved refinement: {report:#?}"
        );
    }
}

#[test]
fn a_guard_can_refine_the_owner_of_a_map_selector() {
    let source = format!(
        "{}
         (defn text-payload?
           {{:t/sig '[(s/or :text string? :counter number?) :-> boolean?]
             :t/guard '[:both x string?]}}
           [x] (string? x))
         (defn use-payload {{:t/sig '[::event :-> string?]}} [event]
           (if (text-payload? (:value event))
             (str/trim (:value event))
             (str (inc (:value event)))))",
        include_str!("../examples/tagged_maps.clj")
    );
    let report = Session::default().check("tagged.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn shadowed_primitives_and_malformed_guard_claims_do_not_prove_facts() {
    for source in [
        EXAMPLE.replace(
            ":t/guard '[:both value string?]",
            ":t/guard [:both value string?]",
        ),
        EXAMPLE.replace("[:both value string?]", "[:both missing string?]"),
        EXAMPLE.replace("(defn text?", "(defn ^:dynamic text?"),
        format!("{EXAMPLE}\n(defn string? [x] true)"),
        format!(
            "{EXAMPLE}\n(defn string? {{:t/sig '[(s/nilable string?) :-> boolean?]}} [x] true)"
        ),
    ] {
        let report = Session::default().check("guards.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX9008"),
            "{report:#?}"
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX2322"),
            "{report:#?}"
        );
    }
}

#[test]
fn changing_only_the_proof_body_rechecks_guard_consumers() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [lib :as lib] [clojure.spec.alpha :as s]))
          (defn use-value {:t/sig '[(s/nilable string?) :-> string?]} [x]
            (if (lib/text? x) (clojure.string/upper-case x) \"none\"))
          (defn independent {:t/sig '[:-> number?]} [] 1)",
    };
    let library = Source {
        path: "lib.clj",
        source: "(ns lib (:require [clojure.spec.alpha :as s]))
          (defn text? {:t/sig '[(s/nilable string?) :-> boolean?]
                       :t/guard '[:both x string?]} [x] (string? x))",
    };
    let mut session = Session::default();
    let first = session.check_project(&[app, library], None);
    assert!(first.ok && first.complete, "{first:#?}");
    let unchanged = session.check_project(&[app, library], None);
    assert_eq!(unchanged.stats.reused_functions, 3);

    let bad = library.source.replace("(string? x)", "true");
    let bad = Source {
        source: &bad,
        ..library
    };
    let updated = session.check_project(&[app, bad], None);
    let clean = Session::default().check_project(&[app, bad], None);
    assert_eq!(updated.files[0].stats.checked_functions, 1);
    assert_eq!(updated.files[0].stats.reused_functions, 1);
    assert_eq!(updated.files[0].diagnostics[0].code, "PNX2322");
    assert_eq!(updated.files[1].diagnostics[0].code, "PNX9008");
    assert_eq!(
        serde_json::to_value(&updated.files[0].diagnostics).unwrap(),
        serde_json::to_value(&clean.files[0].diagnostics).unwrap()
    );
    assert!(session.check_project(&[app, library], None).ok);

    let alternate_core = Source {
        path: "core.clj",
        source: "(ns clojure.core)",
    };
    let shadowed = session.check_project(&[app, library, alternate_core], None);
    assert_eq!(shadowed.files[0].diagnostics[0].code, "PNX2322");
    assert_eq!(shadowed.files[1].diagnostics[0].code, "PNX9008");
    assert!(session.check_project(&[app, library], None).ok);
}
