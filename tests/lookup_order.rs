use parenox::{DocumentHover, Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    Session::default().check(
        "lookup.clj",
        &format!("(defn f {{:t/sig '[:-> {expected}]}} [] {expression})"),
    )
}

#[test]
fn lookup_examples_track_exact_recovery_and_default_result() {
    let mut session = Session::default();
    let report = session.check(
        "lookup_order.clj",
        include_str!("../examples/lookup_order.clj"),
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = session.check(
        "lookup_order_bad.clj",
        include_str!("../examples/lookup_order_bad.clj"),
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some(":default"));
}

#[test]
fn argument_failures_take_precedence_over_lookup_and_later_arguments() {
    for expression in [
        r#"(get (throw (ex-info "target" {:stage :target})) :status (inc "dead"))"#,
        r#"(get (throw (ex-info "target" {:stage :target})) (throw (ex-info "key" {:stage :key})))"#,
        r#"(req! (throw (ex-info "target" {:stage :target})) :missing)"#,
        r#"(contains? (throw (ex-info "target" {:stage :target})) (throw (ex-info "key" {:stage :key})))"#,
        r#"(get-in (throw (ex-info "target" {:stage :target})) [(throw (ex-info "key" {:stage :key}))])"#,
    ] {
        let form = format!("(try {expression} (catch ExceptionInfo e (:stage (ex-data e))))");
        let report = check(&form, "#{:target}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(get (throw nil) unresolved :default)", "nil?");
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2304");
}

#[test]
fn successful_lookups_do_not_add_impossible_exception_paths() {
    for (expression, expected) in [
        ("(get {} :status :missing)", "#{:missing}"),
        ("(get #{:ready} :missing)", "nil?"),
        ("(get-in {:status :ready} [:status])", "#{:ready}"),
        ("(contains? {:status nil} :status)", "#{true}"),
        ("(req! {:status nil} :status)", "nil?"),
    ] {
        let form = format!("(try {expression} (catch Throwable _ 42))");
        let report = check(&form, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check(
        "(try (req! 1 :missing) (catch IllegalArgumentException _ :missing))",
        "#{:missing}",
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2741");
}

#[test]
fn nested_set_paths_keep_present_falsey_members_and_eager_defaults() {
    for (expression, expected) in [
        ("(get-in {:choices #{nil}} [:choices nil])", "nil?"),
        ("(get-in {:choices #{false}} [:choices false])", "#{false}"),
        (
            "(get-in {:choices #{false}} [:choices false] :fallback)",
            "#{false}",
        ),
        (
            "(let [path [:choices nil]] (get-in {:choices #{nil}} path :fallback))",
            "nil?",
        ),
        (
            r#"(try (get-in {:choices #{false}} [:choices false] (throw (ex-info "fallback" {})))
                    (catch ExceptionInfo _ :throws))"#,
            "#{:throws}",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let gap = check("(get-in {:present 1} [[1]] :fallback)", "t/any");
    assert!(!gap.complete);
    assert_eq!(gap.diagnostics[0].code, "PNX9001");
}

#[test]
fn scalar_map_keys_preserve_presence_and_lookup_values() {
    for (expression, expected) in [
        (
            r#"(get {nil "stored" false nil \x 3} nil :missing)"#,
            "string?",
        ),
        ("(contains? {false nil} false)", "#{true}"),
        (
            "(get-in {false {nil :stored}} [false nil] :missing)",
            "#{:stored}",
        ),
        (r#"(let [k \x] (req! {k "stored"} \x))"#, "string?"),
        (r#"(let [{x 7 :or {7 :fallback}} {}] x)"#, "#{:fallback}"),
        (
            "(req! (assoc {false :prior} nil :stored) nil)",
            "#{:stored}",
        ),
        ("(get-in {:present 1} [nil] :fallback)", "#{:fallback}"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn symbol_map_keys_distinguish_evaluation_from_quoted_data() {
    for (expression, expected) in [
        ("(req! {'demo/label :stored} 'demo/label)", "#{:stored}"),
        ("(req! '{label :stored} 'label)", "#{:stored}"),
        ("(get '{'label :stored} 'label :missing)", "#{:missing}"),
        ("(get-in {'label {nil :stored}} ['label nil])", "#{:stored}"),
        (
            "(let [key 'label] (req! {key :stored} 'label))",
            "#{:stored}",
        ),
        ("(contains? {'label nil} 'label)", "#{true}"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn associative_indexed_reads_use_jvm_int_value() {
    for (expression, expected) in [
        ("(get [:first :second] 4294967297)", "#{:second}"),
        (r#"(req! "AB" -4294967295N)"#, "char?"),
        (
            "(get-in {:items [:stored]} [:items 4294967296N] :missing)",
            "#{:stored}",
        ),
        (r#"(contains? "hi" 4294967296N)"#, "#{true}"),
        ("(int? (nth (find [42] 0N) 0))", "#{false}"),
        ("(get {0 :stored} 4294967296 :missing)", "#{:missing}"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn proven_lookup_misses_keep_the_default_collection_evidence() {
    for lookup in [
        r#"(get {:occupied 1} :absent [["x"]])"#,
        r#"(get nil [1 2] [["x"]])"#,
        r#"(get [:occupied] 4 [["x"]])"#,
        r#"(get false [1 2] [["x"]])"#,
        r#"(get #{} :absent [["x"]])"#,
    ] {
        let report = check(
            &format!("(mapv first {lookup})"),
            r#"(clojure.spec.alpha/tuple #{"x"})"#,
        );
        assert!(report.ok && report.complete, "{lookup}: {report:#?}");
    }
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
    (defn run {:t/sig '[(t/map {:opt {:maybe (s/tuple (s/tuple string?))}}) :-> any?]} [row]
      (mapv first (get row :maybe [["fallback"]])))"#;
    let report = Session::default().check("lookup.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001")
    );
}

#[test]
fn a_jvm_number_can_define_its_own_lookup() {
    let source = r#"(defn read-mark {:t/sig '[number? :-> #{:fallback}]} [value]
      (get value :mark :fallback))"#;
    let report = Session::default().check("custom_lookup.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("ILookup"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        ":mark"
    );
}

#[test]
fn a_literal_get_in_lookup_can_throw_before_the_next_key_expression() {
    let source = r#"(ns demo)
        (defn inlined {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (get-in target [:key (throw (ex-info "late key" {}))])
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))
        (defn with-default {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (get-in target [:key (throw (ex-info "late key" {}))] :fallback)
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))
        (defn computed-path {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (let [path [:key (throw (ex-info "late key" {}))]] (get-in target path))
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))"#;
    let report = Session::default().check("lookup.clj", source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some(":later"));
    assert_eq!(diagnostic.actual.as_deref(), Some(":earlier | :later"));
    assert!(diagnostic.message.contains("demo/inlined"));
}

#[test]
fn single_entry_maps_preserve_narrowed_symbol_keys() {
    let source = r#"(ns demo.handlers (:require [clojure.spec.alpha :as s]))
        (def transport-key (str ::file))
        (defn lookup {:t/sig '[:-> #{"reader"}]} []
          (get {transport-key "reader"} transport-key))
        (defn invalid {:t/sig '[(s/nilable #{":demo.handlers/file"}) :-> t/any]} [tag]
          (when (some? tag)
            (let [handlers {tag "reader"}
                  picked (get handlers tag)]
              (inc picked))))
        (defn consume {:t/sig '[{":demo.handlers/file" string?} :-> t/any]} [handlers]
          handlers)
        (defn wrong-handler {:t/sig '[:-> t/any]} []
          (consume {transport-key 42}))"#;
    let mut session = Session::default();
    let report = session.check("handlers.clj", source);
    assert!(report.complete, "{report:#?}");
    let [error, wrong_handler] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "picked"
    );
    assert_eq!(wrong_handler.code, "PNX2322");
    assert_eq!(
        &source[wrong_handler.span.start.offset..wrong_handler.span.end.offset],
        "42"
    );
    let key = source.find("{tag").unwrap() + 1;
    let Some(DocumentHover::Local(local)) = session.document_hover_at("handlers.clj", key) else {
        panic!("Expected a type at the key read");
    };
    assert_eq!(local.inferred_type, "\":demo.handlers/file\"");
}

#[test]
fn single_entry_maps_keep_unknown_keys_and_value_failures_explicit() {
    let source = r#"(ns demo.handlers)
        (defn broad {:t/sig '[string? :-> t/any]} [tag] {tag "reader"})
        (defn failed {:t/sig '[:-> #{:failed}]} []
          (let [tag :file]
            (try {tag (throw (ex-info "bad reader" {}))}
                 (catch ExceptionInfo _ :failed))))"#;
    let report = Session::default().check("handlers.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(&source[gap.span.start.offset..gap.span.end.offset], "tag");
    assert!(!report.complete);
}

#[test]
fn single_entry_map_keys_preserve_class_resolution_and_runtime_rejections() {
    let source = r#"(ns demo.handlers)
        (def end. "reader")
        (def .start "reader")
        (defn class-read {:t/sig '[:-> t/any]} [] {end. 1})
        (defn ordinary-read {:t/sig '[:-> #{:ready}]} [] (get {.start :ready} .start))
        (defn no-result {:t/sig '[:-> t/any]} []
          (let [tag :file
                handlers {tag (subs 42 0)}]
            (inc "unreachable")))"#;
    let report = Session::default().check("handlers.clj", source);
    let [gap, error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(&source[gap.span.start.offset..gap.span.end.offset], "end.");
    assert_eq!(error.code, "PNX2322");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "42"
    );
}

#[test]
fn an_opaque_macro_shadowing_get_in_has_no_standard_lookup_proof() {
    let source = r#"(ns demo)
        (defmacro get-in [& _] :real)
        (defn opaque {:t/sig '[:-> #{:real}]} [] (get-in {} [:missing] :fallback))
        (defn explicit {:t/sig '[:-> #{:fallback}]} [] (clojure.core/get-in {} [:missing] :fallback))"#;
    let report = Session::default().check("lookup.clj", source);
    let [declaration, use_site] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(declaration.code, "PNX9001");
    assert_eq!(use_site.code, "PNX9001");
    assert_eq!(
        &source[use_site.span.start.offset..use_site.span.end.offset],
        "get-in"
    );
}

#[test]
fn a_shadowed_get_in_cannot_narrow_the_map_in_a_branch() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defmacro get-in [& _] true)
        (defn read-path {:t/sig '[{:outer {:leaf (s/nilable string?)}} :-> string?]} [m]
          (if (get-in m [:outer :leaf])
            (let [leaf (:leaf (:outer m))] (subs leaf 0))
            "fallback"))"#;
    let mut session = Session::default();
    let report = session.check("lookup.clj", source);
    let mismatch = report
        .diagnostics
        .iter()
        .find(|finding| finding.code == "PNX2322")
        .unwrap();
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    let offset = source.rfind("leaf 0").unwrap();
    let Some(DocumentHover::Local(local)) = session.document_hover_at("lookup.clj", offset) else {
        panic!("expected a type for the leaf read");
    };
    assert_eq!(local.inferred_type, "Nil | String");
}

#[test]
fn a_source_macro_shadows_core_lookups_after_its_definition() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn early {:t/sig '[:-> #{:present}]} [] (get {:path :present} :path))
        (defmacro get [& _] true)
        (defn late {:t/sig '[{:path (s/nilable string?)} :-> string?]} [m]
          (if (get m :path) (subs (:path m) 0) "fallback"))
        (defn qualified {:t/sig '[:-> #{:present}]} [] (clojure.core/get {:path :present} :path))"#;
    let mut session = Session::default();
    let report = session.check("lookup.clj", source);
    let [declaration, macro_use, mismatch] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(declaration.code, "PNX9001");
    assert_eq!(macro_use.code, "PNX9001");
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    let opaque = source.find("get m :path").unwrap() + "get m ".len();
    assert!(session.document_hover_at("lookup.clj", opaque).is_none());
    let early = source.find(":present} :path").unwrap() + ":present} ".len();
    let Some(DocumentHover::Lookup { inferred_type, .. }) =
        session.document_hover_at("lookup.clj", early)
    else {
        panic!("expected a type before the macro declaration");
    };
    assert_eq!(inferred_type, ":present");
}

#[test]
fn map_writes_wait_for_every_argument_before_validating_the_write() {
    for expression in [
        r#"(assoc (throw (ex-info "target" {:stage :first})) :status (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc {} (throw (ex-info "key" {:stage :first})) (throw (ex-info "later" {:stage :later})) :status (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc 42 :status (throw (ex-info "value" {:stage :first})))"#,
        r#"(assoc-in (throw (ex-info "target" {:stage :first})) [(throw (ex-info "later" {:stage :later}))] (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc-in {} [(throw (ex-info "key" {:stage :first}))] (throw (ex-info "later" {:stage :later})))"#,
    ] {
        let form = format!("(try {expression} (catch ExceptionInfo e (:stage (ex-data e))))");
        let report = check(&form, "#{:first}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(assoc {} :status (throw nil) :later unresolved)", "nil?");
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2304");
}

#[test]
fn invalid_scalar_writes_raise_only_class_cast_exceptions() {
    let report = check(
        "(try (assoc-in {:request 7} [:request :status] :ready)
              (catch ExceptionInfo _ :unexpected)
              (catch ClassCastException _ :wrong-parent))",
        "#{:wrong-parent}",
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Number"));
}
