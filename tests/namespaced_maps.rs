use parenox::{Session, Source};

const SOURCE: &str = include_str!("../examples/namespaced_keys.cljc");

#[test]
fn aliases_qualify_auto_map_keys_under_their_source_namespace() {
    let mut session = Session::default();
    let good = session.check("namespaced_keys.cljc", SOURCE);
    assert!(good.ok && good.complete, "{good:#?}");
    let edited = SOURCE.replace(":_/source", "::payload/sku");
    let bad = session.check("namespaced_keys.cljc", &edited);
    assert!(!bad.complete, "{bad:#?}");
    let [error] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(
        &edited[error.span.start.offset..error.span.end.offset],
        "::payload/sku"
    );
}

#[test]
fn map_alias_errors_remain_visible_when_the_value_is_discarded_or_inactive() {
    for source in [
        "(ns demo)\n#_ #::payload{:sku nil}",
        "(ns demo)\n#?(:clj nil :cljs #::payload{:sku nil})",
        "#::payload{:sku nil}\n(ns demo (:require [catalog.payload :as-alias payload]))",
    ] {
        let report = Session::default().check("namespace.cljc", source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            "payload"
        );
    }
    let externs = Source {
        path: "api.edn",
        source: "#::{name [:-> string?]}",
    };
    let report = Session::default().check_project_with_externs(&[], &[externs]);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[0].diagnostics[0]
            .message
            .contains("Auto-resolved maps")
    );
}
