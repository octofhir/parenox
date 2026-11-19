use parenox::{Session, Source};

const MODEL: Source<'_> = Source {
    path: "model.clj",
    source: r#"(ns model (:require [clojure.spec.alpha :as s]))
(s/def ::id string?)
(s/def :wire/id string?)"#,
};

fn assert_target(
    session: &Session,
    caller: Source<'_>,
    fragment: &str,
    spelling: &str,
    provider: Source<'_>,
    declaration: &str,
) {
    let at = caller.source.find(fragment).unwrap() + fragment.find(spelling).unwrap();
    let target = session
        .definition(caller.path, at + 1)
        .unwrap_or_else(|| panic!("No definition from {fragment}"));
    assert_eq!(target.path, provider.path, "{fragment}");
    let start = provider.source.find(declaration).unwrap();
    assert_eq!(target.span.start.offset, start, "{fragment}");
    assert_eq!(
        &provider.source[start..target.span.end.offset],
        declaration,
        "{fragment}"
    );
}

#[test]
fn navigates_to_the_declaring_file_from_supported_type_positions() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [clojure.spec.alpha :as s] [model :as-alias m]))
(s/def ::record (s/keys :req-un [::m/id]))
(s/def ::label (s/nilable :wire/id))
(s/fdef show :args (s/alt :one (s/cat :id ::m/id) :zero (s/cat)) :ret :wire/id)
(defn show ([] "") ([id] id))
(defn named {:t/sig '[::record :-> ::label]} [record] (:id record))
(defn ^{:t/sig '[:wire/id :-> string?]} metadata [id] id)
(defn generic {:t/sig '(t/all [a] [a ::m/id :-> a])} [x _] x)
(defn multi {:t/sig '([:-> string?] [::m/id :-> string?])} ([] "") ([id] id))"#,
    };
    let mut session = Session::default();
    session.check_project(&[app, MODEL], None);
    for (fragment, spelling, provider, declaration) in [
        ("[::m/id]", "::m/id", MODEL, "::id"),
        ("(s/nilable :wire/id)", ":wire/id", MODEL, ":wire/id"),
        ("(s/cat :id ::m/id)", "::m/id", MODEL, "::id"),
        (":ret :wire/id", ":wire/id", MODEL, ":wire/id"),
        ("'[::record :->", "::record", app, "::record"),
        (":-> ::label]", "::label", app, "::label"),
        ("'[:wire/id :->", ":wire/id", MODEL, ":wire/id"),
        ("[a ::m/id :-> a]", "::m/id", MODEL, "::id"),
        ("[::m/id :-> string?]", "::m/id", MODEL, "::id"),
    ] {
        assert_target(&session, app, fragment, spelling, provider, declaration);
    }
}

#[test]
fn locates_valid_calls_but_not_runtime_data_or_shadowed_calls() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [clojure.spec.alpha :as s :refer [valid?]]))
(s/def ::tag #{:wire/id})
(s/def ::choice (s/or :wire/id string?))
(defn alias [x] (if (s/valid? :wire/id x) x nil))
(defn referred [y] (valid? :wire/id y))
(defn with-form [f] (s/valid? :wire/id f 'input))
(defn shadowed [valid? z] (valid? :wire/id z))
(defn data [] {:wire/id 42})
(defn ordinary [x] (= :wire/id x))
(defn quoted [q] '(s/valid? :wire/id q))
(defn commented [c] (comment (s/valid? :wire/id c)))
#_(defn discarded [d] (s/valid? :wire/id d))
;; :wire/id
(defn literal-shape {:t/sig '[{:wire/id string?} :-> nil?]} [_] nil)"#,
    };
    let mut session = Session::default();
    session.check_project(&[MODEL, app], None);
    for fragment in [
        "(s/valid? :wire/id x)",
        "(valid? :wire/id y)",
        "(s/valid? :wire/id f 'input)",
    ] {
        assert_target(&session, app, fragment, ":wire/id", MODEL, ":wire/id");
    }
    for fragment in [
        "#{:wire/id}",
        "(s/or :wire/id string?)",
        "(valid? :wire/id z)",
        "{:wire/id 42}",
        "(= :wire/id x)",
        "'(s/valid? :wire/id q)",
        "(comment (s/valid? :wire/id c))",
        "#_(defn discarded [d] (s/valid? :wire/id d))",
        ";; :wire/id",
        "{:wire/id string?}",
    ] {
        let at = app.source.find(fragment).unwrap() + fragment.find(":wire/id").unwrap();
        assert_eq!(session.definition(app.path, at), None, "{fragment}");
    }
}

#[test]
fn does_not_treat_project_valid_function_as_the_runtime_spec_operation() {
    let replacement = Source {
        path: "spec.clj",
        source: "(ns clojure.spec.alpha) (defn valid? [_ _] true)",
    };
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [clojure.spec.alpha :as s]))
(defn validate [x] (s/valid? :wire/id x))
(defn checked {:t/sig '[:wire/id :-> string?]} [x] x)"#,
    };
    let mut session = Session::default();
    session.check_project(&[MODEL, app, replacement], None);
    let runtime_argument = app.source.find(":wire/id").unwrap();
    assert_eq!(session.definition(app.path, runtime_argument), None);
    assert_target(
        &session,
        app,
        "'[:wire/id :->",
        ":wire/id",
        MODEL,
        ":wire/id",
    );
}

#[test]
fn current_snapshot_keeps_completed_sites_and_never_picks_a_duplicate() {
    let app = Source {
        path: "app.cljc",
        source: r#"(ns app)
#?(:clj (defn run {:t/sig '[:wire/id :-> string?]} [id] id)
   :cljs (defn hidden {:t/sig '[:wire/id :-> string?]} [id] id))
(defn unfinished ["#,
    };
    let incomplete = Source {
        path: MODEL.path,
        source: "(ns model (:require [clojure.spec.alpha :as s]))\n\n\n(s/def :wire/id string?)\n(",
    };
    let duplicate = Source {
        path: "other.clj",
        source: "(ns other (:require [clojure.spec.alpha :as s])) (s/def :wire/id keyword?)",
    };
    let duplicate_in_file = Source {
        path: MODEL.path,
        source: "(ns model (:require [clojure.spec.alpha :as s])) (s/def :wire/id string?) (s/def :wire/id keyword?)",
    };
    let mut session = Session::default();
    let at = app.source.find(":wire/id").unwrap();
    for (model, extra) in [
        (Some(MODEL), None),
        (Some(incomplete), None),
        (Some(incomplete), Some(duplicate)),
        (Some(duplicate_in_file), None),
        (None, None),
    ] {
        let sources: Vec<_> = [Some(app), model, extra].into_iter().flatten().collect();
        session.check_project(&sources, None);
        if extra.is_none()
            && let Some(model) = model
            && model.source != duplicate_in_file.source
        {
            assert_target(
                &session,
                app,
                "'[:wire/id :->",
                ":wire/id",
                model,
                ":wire/id",
            );
        } else {
            assert_eq!(session.definition(app.path, at), None);
        }
        let inactive = app.source.rfind(":wire/id").unwrap();
        assert_eq!(session.definition(app.path, inactive), None);
    }
}
