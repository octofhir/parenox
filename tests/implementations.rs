use parenox::{Session, Source};

#[test]
fn finds_only_methods_of_the_resolved_source_multimethod() {
    let provider = Source {
        path: "provider.clj",
        source: "(ns provider)\n(defmulti render :kind)\n(defmethod render :local [value] value)\n",
    };
    let other = Source {
        path: "other.clj",
        source: "(ns other)\n(defmulti render :kind)\n(defmethod render :other [value] value)\n",
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns consumer (:require [provider :as p :refer [render] :rename {render render-value}]))
(defmethod p/render :qualified [value] value)
(defmethod render-value :referred [value] value)
(defn use-it [value] (p/render value))
(defn shadow [render-value value] (render-value value))
(defmethod p/render :malformed :not-parameters)
'(defmethod p/render :quoted [value] value)
#_(defmethod p/render :discarded [value] value)
"#,
    };
    let mut session = Session::default();
    session.check_project(&[provider, consumer, other], None);
    let declaration = provider.source.find("render :kind").unwrap();
    let declared = session.implementations(provider.path, declaration);
    let expected = [
        (consumer, "p/render :qualified", "p/render"),
        (consumer, "render-value :referred", "render-value"),
        (provider, "render :local", "render"),
    ];
    assert_eq!(declared.len(), expected.len(), "{declared:#?}");
    for (definition, (file, needle, name)) in declared.iter().zip(expected) {
        let offset = file.source.find(needle).unwrap();
        assert_eq!(definition.path, file.path);
        assert_eq!(definition.span.start.offset, offset);
        assert_eq!(definition.span.end.offset, offset + name.len());
    }
    for usage in [
        "p/render value",
        "p/render :qualified",
        "render-value :referred",
    ] {
        let offset = consumer.source.find(usage).unwrap();
        assert_eq!(
            session.implementations(consumer.path, offset),
            declared,
            "{usage}"
        );
    }
    for non_target in [
        "render-value value))",
        "p/render :quoted",
        "p/render :discarded",
    ] {
        let offset = consumer.source.find(non_target).unwrap();
        assert!(
            session.implementations(consumer.path, offset).is_empty(),
            "{non_target}"
        );
    }
}
