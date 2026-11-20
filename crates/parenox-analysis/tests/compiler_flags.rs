use parenox_analysis::{ExternSnapshot, Session, Source, compare_externs};

const TYPED: &str = "(defn checked {:t/sig '[string? :-> string?]} [x] x)";

#[test]
fn checks_literal_reflection_flags_at_the_start_of_a_namespace() {
    let source = format!(
        "(ns ^:no-doc reflection.flags
  \"Reflection flag fixture.\"
  {{:tags [:test]}}
  (:require [clojure.core :as core]))
; Compiled bodies below have reflection diagnostics enabled.
(set! *warn-on-reflection* true)
#_(println \"discarded\")
(set! clojure.core/*warn-on-reflection* false)
(set! core/*warn-on-reflection* true)
{TYPED}"
    );
    let report = Session::default().check("flags.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn resolves_core_renames_and_reports_excluded_or_foreign_flag_targets() {
    let renamed = format!(
        "(ns reflection.flags (:refer-clojure :rename {{*warn-on-reflection* warn}}))
(set! warn true)
{TYPED}"
    );
    let report = Session::default().check("flags.clj", &renamed);
    assert!(report.ok && report.complete, "{report:#?}");
    for source in [
        "(ns reflection.flags (:refer-clojure :exclude [*warn-on-reflection*]))
(set! *warn-on-reflection* true)",
        "(ns reflection.flags (:require [foreign.core :as core :refer [*warn-on-reflection*]]))
(set! *warn-on-reflection* true)",
        "(ns reflection.flags (:require [foreign.core :as core]))
(set! core/*warn-on-reflection* true)",
        "(ns reflection.flags (:require [foreign.core :refer :all]))
(set! *warn-on-reflection* true)",
    ] {
        let report = Session::default().check("flags.clj", source);
        assert!(!report.complete, "{report:#?}");
        assert!(report.diagnostics.iter().any(|item| item.code == "PNX9001"));
    }
}

#[test]
fn keeps_gaps_for_executable_values_and_other_compiler_flags() {
    for assignment in [
        "(set! *warn-on-reflection* (do (println :effect) true))",
        "(set! *warn-on-reflection* :truthy)",
        "(set! *warn-on-reflection*)",
        "(clojure.core/set! *warn-on-reflection* true)",
        "(set! *unchecked-math* true)",
        "(set! *assert* false)",
    ] {
        let source = format!("(ns reflection.flags)\n{assignment}\n{TYPED}");
        let report = Session::default().check("flags.clj", &source);
        assert!(!report.complete, "{assignment}: {report:#?}");
        assert!(
            report.diagnostics.iter().any(|item| item.code == "PNX9001"),
            "{assignment}: {report:#?}"
        );
    }
}

#[test]
fn rejects_a_namespace_alias_conflict_before_the_flag_assignment() {
    let source = "(ns reflection.flags (:require [clojure.string :as a] [clojure.core :as a]))
(set! a/*warn-on-reflection* true)";
    let report = Session::default().check("flags.clj", source);
    assert!(!report.complete, "{report:#?}");
    for code in ["PNX2790", "PNX9001"] {
        assert!(report.diagnostics.iter().any(|item| item.code == code));
    }
}

#[test]
fn stops_the_proof_at_executable_namespace_metadata_and_prefix_forms() {
    for prefix in [
        "(ns reflection.flags {:opaque (intern *ns* '*warn-on-reflection* 0)})",
        "(ns ^{:opaque (intern *ns* '*warn-on-reflection* 0)} reflection.flags)",
        "(ns reflection.flags (:refer-clojure :invalid true))",
        "(ns reflection.flags (:require))",
        "(ns reflection.flags (:require (clojure)))",
        "(ns reflection.flags (:require [not/a-namespace :as foreign]))",
        "(ns reflection.flags (:refer-clojure :only [not/a-var *warn-on-reflection*]))",
        "(ns first.source)\n(ns reflection.flags)",
        "(ns reflection.flags)\n(def ^:dynamic *warn-on-reflection* false)",
        "(ns reflection.flags)\n(eval '(intern *ns* '*warn-on-reflection* 0))",
        "(ns reflection.flags)\n(def value (do (intern *ns* '*warn-on-reflection* 0) 42))",
    ] {
        let source = format!("{prefix}\n(set! *warn-on-reflection* true)\n{TYPED}");
        let target = source.rfind("(set!").unwrap() + 1;
        let report = Session::default().check("flags.clj", &source);
        assert!(!report.complete, "{prefix}: {report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| { item.code == "PNX9001" && item.span.start.offset == target }),
            "{prefix}: {report:#?}"
        );
    }
}

#[test]
fn refreshes_an_unchanged_file_when_the_project_provides_core() {
    let source = format!("(ns reflection.flags)\n(set! *warn-on-reflection* true)\n{TYPED}");
    let flags = Source {
        path: "flags.clj",
        source: &source,
    };
    let project_core = Source {
        path: "core.clj",
        source: "(ns clojure.core)",
    };
    let mut session = Session::default();
    let before = session.check_project(&[flags], None);
    assert!(before.ok && before.complete, "{before:#?}");
    let with_core = session.check_project(&[flags, project_core], None);
    assert!(!with_core.complete, "{with_core:#?}");
    let diagnostics = &with_core.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 1, "{with_core:#?}");
    assert_eq!(diagnostics[0].code, "PNX9001");
    assert_eq!(diagnostics[0].span.start.line, 2);
    let after = session.check_project(&[flags], None);
    assert!(after.ok && after.complete, "{after:#?}");
    assert_eq!(after.stats.reused_parses, 1);
}

#[test]
fn refreshes_function_findings_when_the_declaration_gap_changes() {
    let source = Source {
        path: "flags.clj",
        source: "(ns reflection.flags (:require [vendor :as v] [clojure.string :as str]))
(set! *warn-on-reflection* true)
(defn checked {:t/sig '[:-> string?]} [] (str/upper-case (v/value)))",
    };
    let project_core = Source {
        path: "core.clj",
        source: "(ns clojure.core)",
    };
    let externs = Some(Source {
        path: "externs.edn",
        source: "{vendor/value [:-> any?]}",
    });
    let mut session = Session::default();
    for (sources, expected) in [
        (&[source][..], "PNX9004"),
        (&[source, project_core][..], "PNX9001"),
        (&[source][..], "PNX9004"),
    ] {
        let report = session.check_project(sources, externs);
        let diagnostics = &report.files[0].diagnostics;
        assert_eq!(diagnostics.len(), 1, "{report:#?}");
        assert_eq!(diagnostics[0].code, expected, "{report:#?}");
        assert_eq!(report.files[0].stats.checked_functions, 1, "{report:#?}");
    }
}

#[test]
fn uses_the_same_flag_proof_when_comparing_externs() {
    let source = format!("(ns reflection.flags)\n(set! *warn-on-reflection* true)\n{TYPED}");
    let flags = Source {
        path: "flags.clj",
        source: &source,
    };
    let project_core = Source {
        path: "core.clj",
        source: "(ns clojure.core)",
    };
    let before_sources = [flags];
    let after_sources = [flags, project_core];
    let snapshot = |sources| ExternSnapshot {
        sources,
        externs: &[],
    };
    let diff = compare_externs(snapshot(&before_sources), snapshot(&before_sources));
    assert!(diff.complete && diff.compatible, "{diff:#?}");
    let replaced = compare_externs(snapshot(&before_sources), snapshot(&after_sources));
    assert!(!replaced.complete, "{replaced:#?}");
}
