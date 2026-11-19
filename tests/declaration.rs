use parenox::{Session, Source};

#[test]
fn imports_have_local_declaration_sites_separate_from_external_definitions() {
    let caller = r#"(ns caller (:require [api :as a]
                      [palette :refer [orange peach] :rename {peach renamed-peach}]
                      [extras :refer :all :exclude [excluded]]
                      fully-qualified))
(defn example []
  (do (a/read!) (orange) (renamed-peach) (extra) (fully-qualified/start)))
(defn shadows [] (let [orange identity] (orange "shadowed")))
'(a/read! "quoted")
a/unknown-provider-var
(excluded)
"#;
    let mut session = Session::default();
    session.check_project(
        &[
            Source {
                path: "caller.clj",
                source: caller,
            },
            Source {
                path: "api.clj",
                source: "(ns api) (defn read! [] nil)",
            },
            Source {
                path: "palette.clj",
                source: "(ns palette) (defn orange [] nil) (defn peach [] nil)",
            },
            Source {
                path: "extras.clj",
                source: "(ns extras) (defn extra [] nil) (defn excluded [] nil)",
            },
            Source {
                path: "fully_qualified.clj",
                source: "(ns fully-qualified) (defn start [] nil)",
            },
        ],
        None,
    );
    for (usage, introduced) in [
        ("(a/read!)", "a"),
        ("(orange)", "orange"),
        ("(renamed-peach)", "renamed-peach"),
        ("(extra)", "extras"),
        ("(fully-qualified/start)", "fully-qualified"),
        ("a/unknown-provider-var", "a"),
    ] {
        let offset = caller.find(usage).unwrap() + usize::from(usage.starts_with('('));
        let declaration = session
            .declaration("caller.clj", offset)
            .unwrap_or_else(|| panic!("{usage}"));
        assert_eq!(declaration.path, "caller.clj");
        assert_eq!(
            &caller[declaration.span.start.offset..declaration.span.end.offset],
            introduced,
        );
        assert!(declaration.span.start.offset < caller.find("(defn example").unwrap());
    }
    let api = caller.find("(a/read!)").unwrap() + 1;
    assert_eq!(
        session.definition("caller.clj", api).unwrap().path,
        "api.clj"
    );
    for usage in ["orange \"shadowed", "a/read! \"quoted", "excluded)"] {
        let offset = caller.find(usage).unwrap();
        assert!(
            session.declaration("caller.clj", offset).is_none(),
            "{usage}"
        );
    }
}

#[test]
fn declarations_follow_the_active_reader_branch_and_completed_namespace_prefix() {
    let source = r#"(ns client
  (:require #?(:clj [api :as a] :cljs [runtime.api :as a])))
(defn run [] #?(:clj (a/read!) :cljs (a/run!)))
(defn unfinished [
"#;
    let mut session = Session::default();
    session.check("client.cljc", source);
    let active = session
        .declaration("client.cljc", source.find("a/read!").unwrap())
        .unwrap();
    assert_eq!(active.span.start.offset, source.find("a] :cljs").unwrap());
    assert!(
        session
            .declaration("client.cljc", source.find("a/run!").unwrap())
            .is_none()
    );
}
