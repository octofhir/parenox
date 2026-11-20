use parenox_analysis::{Session, Source, namespace_documentation};

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
