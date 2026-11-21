use parenox_analysis::{DocumentHover, Session, Source, namespace_documentation};

#[test]
fn local_hover_uses_reachable_iterations_and_lexical_scope() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn revisit {:t/sig '[:-> (s/nilable string?)]} []
  (loop [again? true value nil]
    (identity value)
    (if again? (recur false "ready") value)))
(defn unannotated [value]
  (when (string? value)
    (identity value)
    (let [value :shadow] (identity value))))
(defn opaque [value]
  (when (string? value) (external.scope/maybe-bind value)))
"#;
    let mut session = Session::default();
    session.check("demo.clj", source);
    let ty = |offset| {
        let DocumentHover::Local(local) = session.document_hover_at("demo.clj", offset).unwrap()
        else {
            panic!("expected local hover at {offset}");
        };
        local.inferred_type
    };
    let reads: Vec<_> = source
        .match_indices("identity value")
        .map(|(start, _)| start + "identity ".len())
        .collect();
    assert_eq!(ty(reads[0]), r#"Nil | "ready""#);
    assert_eq!(ty(source.find("value nil").unwrap()), r#"Nil | "ready""#);
    assert_eq!(ty(reads[1]), "String");
    assert_eq!(ty(reads[2]), ":shadow");
    let unknown_scope = source.rfind("value").unwrap();
    assert!(
        session
            .document_hover_at("demo.clj", unknown_scope)
            .is_none()
    );
}

#[test]
fn repeated_hover_uses_current_externs_with_unchanged_source() {
    let consumer = Source {
        path: "client.clj",
        source: r#"(ns client (:require [wire :as w]))
          (defn read-path {:t/sig '[:-> t/any]} []
            (let [path (:path (w/location))] path))"#,
    };
    let mut session = Session::default();
    let key = consumer.source.find(":path").unwrap();
    let binding = consumer.source.find("path (:path").unwrap();
    let read = consumer.source.rfind("path").unwrap();
    for (externs, expected) in [
        ("{wire/location [:-> {:path string?}]}", Some("String")),
        ("{wire/location [:-> {:path number?}]}", Some("Number")),
        ("{}", None),
        ("{wire/location [:-> {:path string?}]}", Some("String")),
    ] {
        session.check_project(
            &[consumer],
            Some(Source {
                path: "wire.edn",
                source: externs,
            }),
        );
        for position in [
            key,
            key + 1,
            read,
            read + 1,
            binding,
            binding + 1,
            key,
            read,
        ] {
            let ty = match session.document_hover_at(consumer.path, position) {
                Some(DocumentHover::Lookup { inferred_type, .. }) => Some(inferred_type),
                Some(DocumentHover::Local(local)) => Some(local.inferred_type),
                Some(DocumentHover::Var(_)) => panic!("expected a value occurrence"),
                None => None,
            };
            assert_eq!(ty.as_deref(), expected, "{externs} at byte {position}");
        }
    }
}

#[test]
fn local_hover_withholds_types_from_unchecked_loop_iterations() {
    let branches: String = (0..16)
        .map(|step| {
            let value = if step == 15 { "\"done\"" } else { "value" };
            format!(":step-{step} (recur :step-{} {value})\n", step + 1)
        })
        .collect();
    let source = format!(
        "(ns demo)
         (defn explore {{:t/sig '[:-> string?]}} []
           (loop [state :step-0 value :ready]
             (str value)
             (case state {branches} value)))"
    );
    let offset = source.find("str value").unwrap() + "str ".len();
    let mut session = Session::default();
    let report = session.check("demo.clj", &source);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PNX9001" && diagnostic.message.contains("recur type inference limit")
    }));
    assert!(session.document_hover_at("demo.clj", offset).is_none());
    let binding = source.find("value :ready").unwrap();
    assert!(session.document_hover_at("demo.clj", binding).is_none());
}

#[test]
fn hovers_typed_and_untyped_workspace_vars_at_declarations_and_uses() {
    let provider = r#"
(ns provider)
(defn greet "Greets a friend."
  {:doc "Greets one or more friends."}
  ([name] (str "Hi " name))
  ([name & others] (str name others)))
(defn checked "An annotated identity." {:t/sig '[string? :-> string?]} [value] value)
(defn- internal "Local helper." [{:keys [id]}] id)
(defn use-internal [x] (internal x))
(defn use-shadow [internal x] (internal x))
"#;
    let consumer = r#"
(ns consumer (:require [provider :as p :refer [greet checked]]))
(defn uses [x] (greet x) (p/checked x) (p/internal x))
(greet "outside a defn")
"#;
    let mut session = Session::default();
    session.check_project(
        &[
            Source {
                path: "provider.clj",
                source: provider,
            },
            Source {
                path: "consumer.clj",
                source: consumer,
            },
        ],
        None,
    );
    let greet = consumer.find("(greet x)").unwrap() + 1;
    let hover = session.hover_at("consumer.clj", greet).unwrap();
    assert_eq!(hover.info.name, "provider/greet");
    assert!(hover.info.contract.is_none());
    assert_eq!(
        hover.documentation.as_deref(),
        Some("Greets one or more friends.")
    );
    assert_eq!(hover.arglists, ["[name]", "[name & others]"]);

    let checked = consumer.find("p/checked").unwrap();
    let hover = session.hover_at("consumer.clj", checked).unwrap();
    assert_eq!(hover.arglists, ["[value]"]);
    assert_eq!(
        hover.documentation.as_deref(),
        Some("An annotated identity.")
    );
    assert_eq!(hover.info.signatures, ["[String -> String]"]);
    assert!(hover.info.contract.is_some());
    let span = session.hover_symbol_span("consumer.clj", checked).unwrap();
    assert_eq!(&consumer[span.start.offset..span.end.offset], "p/checked");

    let private = provider.find("(internal x)").unwrap() + 1;
    let hover = session.hover_at("provider.clj", private).unwrap();
    assert_eq!(hover.documentation.as_deref(), Some("Local helper."));
    assert_eq!(hover.arglists, ["[{:keys [id]}]"]);
    let shadow = provider.rfind("(internal x)").unwrap() + 1;
    assert!(session.hover_at("provider.clj", shadow).is_none());
    let inaccessible = consumer.find("p/internal").unwrap();
    assert!(session.hover_at("consumer.clj", inaccessible).is_none());
    assert!(
        session
            .hover_archive_var("consumer.clj", inaccessible)
            .is_none()
    );

    let declaration = provider.find("greet").unwrap();
    let hover = session.hover_at("provider.clj", declaration).unwrap();
    assert_eq!(hover.info.name, "provider/greet");
    let top_level = consumer.rfind("(greet").unwrap() + 1;
    let hover = session.hover_at("consumer.clj", top_level).unwrap();
    assert_eq!(hover.info.name, "provider/greet");
}

#[test]
fn extracts_def_macro_and_protocol_metadata_without_evaluating_source() {
    let provider = r#"
(ns provider)
(def ^{:doc "Metadata docs." :arglists '([input])} configured 3)
(def value "Value docs." 42)
(defmacro wrapper "Wraps code." [code & extras] code)
(defprotocol Operation "Operations on values."
  (apply-value [this] [this argument] "Runs the operation."))
(defn- private-value "Hidden." [] nil)
"#;
    for (name, documentation, arglists) in [
        ("configured", "Metadata docs.", &["[input]"][..]),
        ("value", "Value docs.", &[][..]),
        ("wrapper", "Wraps code.", &["[code & extras]"][..]),
        ("Operation", "Operations on values.", &[][..]),
        (
            "apply-value",
            "Runs the operation.",
            &["[this]", "[this argument]"][..],
        ),
    ] {
        let docs = namespace_documentation(provider, "provider", name, false).unwrap();
        assert_eq!(docs.documentation.as_deref(), Some(documentation));
        assert_eq!(docs.arglists, arglists);
    }
    assert!(namespace_documentation(provider, "provider", "private-value", false).is_none());
    assert!(namespace_documentation(provider, "wrong-namespace", "value", false).is_none());
    assert!(
        namespace_documentation(
            &format!("{provider}\n(def value 1)"),
            "provider",
            "value",
            false
        )
        .is_none()
    );
}

#[test]
fn sticks_to_lexical_references_for_archive_hover() {
    let source = r#"
(ns client (:require [example.lib :as l :refer [provided]]))
(defn ordinary [x] (l/declared x) (provided x) (map str x))
(defn shadow [provided map x] (provided x) (map x))
(defn quoted [] '(l/declared))
"#;
    let mut session = Session::default();
    session.check("client.clj", source);
    let lookup = |needle: &str, offset| {
        session.hover_archive_var("client.clj", source.find(needle).unwrap() + offset)
    };
    assert_eq!(lookup("l/declared", 0), Some(("example.lib", "declared")));
    assert_eq!(lookup("(provided x)", 1), Some(("example.lib", "provided")));
    assert_eq!(lookup("(map str", 1), Some(("clojure.core", "map")));
    assert_eq!(
        session.hover_archive_var("client.clj", source.rfind("(provided x)").unwrap() + 1,),
        None
    );
    assert_eq!(lookup("(map x)", 1), None);
    assert_eq!(lookup("'(l/declared)", 2), None);
}

#[test]
fn resolves_literal_standalone_require_aliases_at_hover_time() {
    let client = r#"
(ns client)
(helpers/greet "before require")
(require '[provider :as helpers] '[other.library :as archived])
(helpers/greet "after require")
(archived/render "after require")
"#;
    let mut session = Session::default();
    session.check_project(
        &[
            Source {
                path: "provider.clj",
                source: "(ns provider) (defn greet \"Workspace greeting.\" [x] x)",
            },
            Source {
                path: "client.clj",
                source: client,
            },
        ],
        None,
    );
    let before = client.find("helpers/greet").unwrap();
    assert!(session.hover_at("client.clj", before).is_none());
    let after = client.rfind("helpers/greet").unwrap();
    let hover = session.hover_at("client.clj", after).unwrap();
    assert_eq!(hover.info.name, "provider/greet");
    assert_eq!(hover.documentation.as_deref(), Some("Workspace greeting."));
    let archive = client.find("archived/render").unwrap();
    assert_eq!(
        session.hover_archive_var("client.clj", archive),
        Some(("other.library", "render"))
    );
}

#[test]
fn hovers_reached_callback_bindings_by_their_written_occurrence() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn paths
          {:t/sig '[(s/coll-of {:path string?} :kind vector?) :-> any?]}
          [items]
          (let [read-path (fn [{:keys [^String path] :as entry}] [path entry])]
            (mapv read-path items)))
        (defn empty-input {:t/sig '[:-> any?]} []
          (mapv (fn [unvisited] unvisited) []))
        (defn shadowed {:t/sig '[string? :-> number?]} [path]
          (let [path 42] (inc path)))"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let hover = |marker: &str| {
        let offset = source.find(marker).unwrap();
        match session.document_hover_at("demo.clj", offset) {
            Some(DocumentHover::Local(local)) => Some(local.inferred_type),
            Some(_) => panic!("expected local hover at {marker}"),
            None => None,
        }
    };
    assert_eq!(hover("path] :as").as_deref(), Some("String"));
    assert_eq!(hover("entry}]").as_deref(), Some("{:path: String}"));
    assert_eq!(hover("path]\n          (let").as_deref(), Some("String"));
    assert_eq!(hover("path 42").as_deref(), Some("42"));
    assert_eq!(hover("unvisited]"), None);
}

#[test]
fn direct_let_visits_defaults_before_a_nonreturning_initializer() {
    let source = r#"(ns demo)
        (defn cancelled {:t/sig '[:-> #{:cancelled}]} []
          (try
            (let [{:keys [key]
                   :or {key (let [visited "default"] visited)}}
                  (throw (ex-info "cancelled" {}))]
              key)
            (catch clojure.lang.ExceptionInfo error :cancelled)))"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("visited \"default\"").unwrap();
    let Some(DocumentHover::Local(default)) = session.document_hover_at("demo.clj", offset) else {
        panic!("missing hover in the reached default initializer");
    };
    assert_eq!(default.inferred_type, "\"default\"");
    let binding = source.find("key]").unwrap();
    assert!(session.document_hover_at("demo.clj", binding).is_none());
}

#[test]
fn the_first_throwing_default_stops_later_defaults_and_the_initializer() {
    let source = r#"(ns demo)
        (defn stopped {:t/sig '[:-> #{:first}]} []
          (try
            (let [{:keys [bound]
                   :or {unused (throw (ex-info "first" {:step :first}))
                        bound (let [unvisited "second"]
                                (throw (ex-info unvisited {:step :second})))}}
                  (throw (ex-info "initializer" {:step :initializer}))]
              bound)
            (catch clojure.lang.ExceptionInfo error (:step (ex-data error)))))"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("unvisited \"second\"").unwrap();
    assert!(session.document_hover_at("demo.clj", offset).is_none());
}

#[test]
fn a_throwing_default_stops_binding_even_for_a_present_key() {
    let source = r#"(ns demo)
        (defn stopped {:t/sig '[:-> #{:caught}]} []
          (try
            (let [{:keys [present]
                   :or {present (throw (ex-info "eager default" {}))}}
                  {:present "present key"}
                  later 1]
              [present later])
            (catch clojure.lang.ExceptionInfo error :caught)))"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for marker in ["present]", "later 1", "present later", "later])"] {
        let offset = source.find(marker).unwrap();
        assert!(session.document_hover_at("demo.clj", offset).is_none());
    }
}

#[test]
fn a_throwing_binding_does_not_return_nil_from_an_empty_body() {
    let source = r#"(ns demo)
        (defn conditional {:t/sig '[:-> #{:caught}]} []
          (try
            (when-let [{:keys [x] :or {x (throw (ex-info "default" {}))}} {:x :present}])
            (catch clojure.lang.ExceptionInfo _ :caught)))
        (defn callback {:t/sig '[:-> #{:caught}]} []
          (try
            ((fn [{:keys [x] :or {x (throw (ex-info "default" {}))}}]) {:x :present})
            (catch clojure.lang.ExceptionInfo _ :caught)))"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn keyword_callback_hover_joins_reached_lookups_at_the_literal() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn search {:t/sig '[:-> #{:hit}]} []
          (some :result [{:result nil} {:result false} {:result :hit} {:result :unvisited}]))
        (defn project {:t/sig '[:-> (s/tuple string? number?)]} []
          ((juxt :left :right) {:left "left" :right 42}))
        (defn no-items {:t/sig '[:-> nil?]} []
          (some :empty []))
        (defn computed {:t/sig '[:-> #{:result}]} []
          (let [key :dynamic] (some key [{:dynamic :result}])))"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let hover = |marker: &str| {
        let offset = source.find(marker).unwrap();
        match session.document_hover_at("demo.clj", offset) {
            Some(DocumentHover::Lookup {
                key,
                inferred_type,
                span,
            }) => {
                assert_eq!(key, &source[span.start.offset..span.end.offset]);
                Some(inferred_type)
            }
            Some(_) => panic!("expected lookup at {marker}"),
            None => None,
        }
    };
    assert_eq!(hover(":result [{").as_deref(), Some("Nil | false | :hit"));
    assert_eq!(hover(":left :right").as_deref(), Some("\"left\""));
    assert_eq!(hover(":right) {:left").as_deref(), Some("42"));
    assert_eq!(hover(":result :hit"), None);
    assert_eq!(hover(":empty []"), None);
    assert_eq!(hover(":dynamic]"), None);
}
