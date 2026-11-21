use parenox::{DocumentHover, Session, Source};

const SOURCE: &str = include_str!("../examples/jvm_strings.cljc");

#[test]
fn join_accepts_seqable_values_and_reports_the_invalid_collection_input() {
    let source = r#"(ns labels (:require [clojure.string :as str] [clojure.spec.alpha :as s]))
        (defn entries {:t/sig '[map? :-> string?]} [m] (str/join :entry m))
        (defn characters {:t/sig '[string? :-> string?]} [text] (str/join \/ text))
        (defn no-items {:t/sig '[:-> string?]} [] (str/join nil))
        (defn maybe-items {:t/sig '[(s/nilable number?) :-> string?]} [value] (str/join value))
        (defn without-types [] (str/join \/ 42))"#;
    let report = Session::default().check("labels.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .collect();
    assert_eq!(errors.len(), 2, "{report:#?}");
    for (error, target) in errors.into_iter().zip(["value", "42"]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(error.expected.as_deref(), Some("Seqable or Nil"));
        assert_eq!(error.actual.as_deref(), Some("Number"));
        assert_eq!(error.contract.as_ref().unwrap().name, "clojure.string/join");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            target
        );
    }
}

#[test]
fn char_at_checks_utf16_bounds_in_unannotated_bodies() {
    let source = r#"(ns members)
        (defn exact {:t/sig '[:-> #{\a}]} [] (. "😀a" (charAt 2)))
        (defn surrogate {:t/sig '[:-> char?]} [] (.charAt "😀" 0))
        (defn negative {:t/sig '[string? :-> char?]} [text] (.charAt text -1))
        (defn outside [] (long (.charAt "😀" 2)))
        (defn guarded [arg] (when arg (.charAt arg 2)))"#;
    let report = Session::default().check("members.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .collect();
    assert_eq!(errors.len(), 2, "{report:#?}");
    for (error, text) in errors.into_iter().zip(["-1", "2"]) {
        assert_eq!(error.code, "PNX2801");
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            "java.lang.String/charAt"
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            text
        );
    }
}

#[test]
fn substring_uses_utf16_bounds_and_marks_the_invalid_index() {
    let source = r#"(ns captures)
        (defn suffix {:t/sig '[:-> #{"a"}]} [] (subs "😀a" 2))
        (defn prefix {:t/sig '[:-> #{"😀"}]} [] (subs "😀a" 0 2))
        (defn half {:t/sig '[:-> string?]} [] (subs "😀a" 0 1))"#;
    let mut session = Session::default();
    let report = session.check("members.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let invalid = r#"(ns captures (:refer-clojure :rename {subs piece}))
        (defn outside [] (piece "😀" 3))
        (defn reversed [text] (piece text 4 2))
        (defn wrong [] (piece "abc" "0"))"#;
    let report = session.check("members.cljc", invalid);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    assert_eq!(errors.len(), 3, "{report:#?}");
    for (finding, (code, target)) in
        errors
            .into_iter()
            .zip([("PNX2801", "3"), ("PNX2801", "2"), ("PNX2322", "\"0\"")])
    {
        assert_eq!(finding.code, code);
        assert_eq!(
            &invalid[finding.span.start.offset..finding.span.end.offset],
            target
        );
    }
}

#[test]
fn rejected_substring_operands_do_not_supply_a_result_or_a_bounds_error() {
    let source = r#"(ns captures)
        (defn invalid-text {:t/sig '[:-> t/any]} [] (inc (subs nil -1)))
        (defn invalid-index {:t/sig '[:-> t/any]} [] (inc (subs "abc" \a)))
        (defn nested-call {:t/sig '[:-> t/any]} []
          (inc (subs (subs nil 0) :bad-index)))
        (defn partial-text {:t/sig '[boolean? :-> t/any]} [flag]
          (inc (subs (if flag "abc" nil) 0)))"#;
    let mut session = Session::default();
    let report = session.check("members.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        errors,
        [
            ("PNX2322", "nil"),
            ("PNX2322", "\\a"),
            ("PNX2322", "nil"),
            ("PNX2322", ":bad-index"),
            ("PNX2322", "(subs (if flag \"abc\" nil) 0)"),
            ("PNX2322", "(if flag \"abc\" nil)"),
        ],
        "{report:#?}"
    );
}

#[test]
fn a_rejected_substring_chain_keeps_assertion_coverage() {
    let source = r#"(ns captures (:require [clojure.test :refer [deftest is]]))
        (deftest bad-substring (is (= "a" (subs (subs false 0) 1))))"#;
    let report = Session::default().check("members.clj", source);
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn unannotated_substring_checks_a_locally_computed_utf16_endpoint() {
    let source = r#"
(ns arglist)
(defn contents []
  (let [fn-def {:arglist-strs ["[🦊]"]}
        raw-arglist (get (:arglist-strs fn-def) 0)]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
(defn shadowed [count]
  (subs "x" 1 (dec (count "x"))))
(defn dead []
  (if false (subs "x" 1 (dec (count "x"))) :skip))
"#;
    let broken = source.replace("[🦊]", "x");
    let repaired = source.replace("[🦊]", "[]");
    let mut session = Session::default();
    for (text, expected) in [(source, 0), (&broken, 1), (&repaired, 0), (source, 0)] {
        let report = session.check("arglist.clj", text);
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/subs");
            assert_eq!(
                &text[finding.span.start.offset..finding.span.end.offset],
                "(dec (count raw-arglist))"
            );
        }
    }
}

#[test]
fn unannotated_arglist_reads_keep_ordinary_vector_elements() {
    let source = r#"
(ns demo (:require [arglists :as provider]))
(defn local-arglist []
  (let [fn-def {:arglist-strs (first [["[🦊]"]])}
        raw-arglist (first (:arglist-strs fn-def))
        arglist-contents (subs raw-arglist 1 (dec (count raw-arglist)))]
    (re-matches #".*[\[\{].*" arglist-contents)))
(defn structural-arglist []
  (let [raw-arglist (first (provider/arglists))]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
(defn shadowed [first]
  (let [raw-arglist (first ["x"])]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
"#;
    let externs = Source {
        path: "externs.edn",
        source: r#"{arglists/arglists [:-> (clojure.spec.alpha/tuple #{"x"})]}"#,
    };
    let broken = source.replace("[🦊]", "x");
    let mut session = Session::default();
    for (source, expected) in [(source, 0), (broken.as_str(), 1), (source, 0)] {
        let report = session.check_project(
            &[Source {
                path: "src/demo.clj",
                source,
            }],
            Some(externs),
        );
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report.files[0]
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(finding.span.start.line, 6);
            assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/subs");
            assert_eq!(
                &source[finding.span.start.offset..finding.span.end.offset],
                "(dec (count raw-arglist))"
            );
        }
    }
}

#[test]
fn unannotated_line_slice_checks_the_first_invalid_bound() {
    let source = r#"
(defn alias-text []
  (let [lines (nth [["a🦊b"]] (dec 1))
        name-end-row 1
        name-end-col 2
        alias-col 5]
    (subs (nth lines (dec name-end-row))
          (dec name-end-col) (dec alias-col))))
(defn index-only [] (nth [] 3))
"#;
    let broken_column = source.replace("alias-col 5", "alias-col 6");
    let broken_row = broken_column.replace("name-end-row 1", "name-end-row 2");
    let mut session = Session::default();
    for (text, expected) in [
        (source, None),
        (
            &broken_column,
            Some(("clojure.core/subs", "(dec alias-col)")),
        ),
        (
            &broken_row,
            Some(("clojure.core/nth", "(dec name-end-row)")),
        ),
        (source, None),
    ] {
        let report = session.check("aliases.clj", text);
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        match (errors.as_slice(), expected) {
            ([], None) => {}
            ([finding], Some((origin, fragment))) => {
                assert_eq!(finding.code, "PNX2801");
                assert_eq!(finding.contract.as_ref().unwrap().name, origin);
                assert_eq!(
                    &text[finding.span.start.offset..finding.span.end.offset],
                    fragment
                );
            }
            _ => panic!("{report:#?}"),
        }
    }
}

#[test]
fn multiline_alias_text_checks_the_middle_slice_before_the_last_line() {
    let source = r#"(ns aliases (:require [clojure.string :as string]))
(defn between []
  (let [lines ["name" "  :as-alias" "  ; 🦊" "alias"]
        name-end-row 1 name-end-col 5 alias-row 4 alias-col 3]
    (str
      (subs (nth lines (dec name-end-row)) (dec name-end-col))
      "\n"
      (string/join "\n" (subvec lines name-end-row (dec alias-row)))
      "\n"
      (subs (nth lines (dec alias-row)) 0 (dec alias-col)))))"#;
    let broken = source.replace("alias-row 4", "alias-row 6");
    let mut session = Session::default();
    for (text, expected) in [(source, 0), (broken.as_str(), 1), (source, 0)] {
        let report = session.check("aliases.clj", text);
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(
                finding.contract.as_ref().unwrap().name,
                "clojure.core/subvec"
            );
            assert_eq!(
                &text[finding.span.start.offset..finding.span.end.offset],
                "(dec alias-row)"
            );
            assert_eq!(finding.span.start.line, 8);
        }
    }
}

#[test]
fn literal_rendering_checks_version_labels_and_nil_without_a_catch_path() {
    let source = r#"
(defn version-label {:t/sig '[:-> #{"2026.11-SNAPSHOT (🦊12345)"}]} []
  (let [version "2026.11-SNAPSHOT" revision "🦊123456789"]
    (str version " (" (subs revision 0 (min 7 (count revision))) ")")))
(defn pure {:t/sig '[:-> #{"a🦊"}]} []
  (try (str "a" nil "🦊") (catch Throwable _ :render-failed)))
(defn empty-label {:t/sig '[:-> #{""}]} [] (str))
(defn fixed-spread {:t/sig '[:-> #{"a🦊"}]} [] (apply str ["a" nil "🦊"]))
"#;
    let report = Session::default().check("labels.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn rendering_caps_literals_and_preserves_effects_of_unknown_arguments() {
    let astral = "🦊".repeat(256);
    let source = format!(
        r#"(ns labels (:require [clojure.spec.alpha :as s]))
(defn boundary {{:t/sig '[:-> string?]}} []
  (let [boundary (str "{astral}" nil)] boundary))
(defn over-limit {{:t/sig '[:-> string?]}} []
  (let [over-limit (str "{astral}" "a")] over-limit))
(defn uncertain {{:t/sig '[any? :-> any?]}} [object]
  (let [uncertain (try (str "{astral}" "a" object)
                       (catch Throwable _ :render-failed))] uncertain))
(defn open-spread {{:t/sig '[(s/coll-of #{{"a"}} :kind vector?) :-> string?]}} [pieces]
  (let [joined (apply str pieces)] joined))"#,
    );
    let mut session = Session::default();
    let report = session.check("render.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (marker, expected) in [
        ("boundary))", format!("\"{astral}\"")),
        ("over-limit))", "String".into()),
        ("uncertain))", "String | :render-failed".into()),
        ("joined))", "String".into()),
    ] {
        let offset = source.find(marker).unwrap();
        let Some(DocumentHover::Local(hover)) = session.document_hover_at("render.clj", offset)
        else {
            panic!("missing hover for {marker}");
        };
        assert_eq!(hover.inferred_type, expected);
    }
}

#[test]
fn structural_traversal_does_not_supply_literal_str_arguments() {
    let source = r#"(ns rendering (:require [clojure.spec.alpha :as s]))
(defn projected {:t/sig '[(s/tuple #{"x"}) :-> any?]} [parts]
  (let [joined (apply str parts)
        seq-joined (apply str (seq parts))
        prefixed (apply (partial str "prefix:") parts)
        mapped (mapv (fn [part] (str part "!")) parts)]
    [joined seq-joined prefixed mapped]))"#;
    let mut session = Session::default();
    let report = session.check("render.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 4, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001" && finding.severity == "warning")
    );
    let start = source.find("[joined seq-joined").unwrap();
    for name in ["joined", "seq-joined", "prefixed", "mapped"] {
        let offset = start + source[start..].find(name).unwrap();
        let hover = session.document_hover_at("render.clj", offset);
        if name == "mapped" {
            let Some(DocumentHover::Local(hover)) = hover else {
                panic!("missing vector result");
            };
            assert_eq!(hover.inferred_type, "Vector<Unknown>");
        } else {
            assert!(hover.is_none(), "unexpected hover for {name}");
        }
    }
    assert!(
        session
            .document_hover_at("render.clj", source.find("part \"!\"").unwrap())
            .is_none()
    );
}

#[test]
fn checks_jvm_utf16_length_and_characters_after_nil_guards() {
    let mut session = Session::default();
    let report = session.check("jvm_strings.cljc", SOURCE);
    assert!(report.ok && report.complete, "{report:#?}");
    let editable = SOURCE.replace("(when name", "(do");
    let report = session.check("jvm_strings.cljc", &editable);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        error.contract.as_ref().unwrap().name,
        "java.lang.String/charAt"
    );
    assert_eq!(
        &editable[error.span.start.offset..error.span.end.offset],
        "name"
    );
}

#[test]
fn checks_member_arguments_and_marks_other_receivers_incomplete() {
    let source = r#"(defn indexed {:t/sig '[string? :-> char?]} [text] (.charAt text "0"))
         (defn unknown {:t/sig '[map? :-> number?]} [data] (.length data))"#;
    let report = Session::default().check("members.clj", source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert_eq!(report.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(report.diagnostics[0].severity, "error");
    assert_eq!(report.diagnostics[1].severity, "warning");
    assert!(!report.complete);
}

#[test]
fn string_search_members_infer_boolean_and_numeric_results() {
    let source = r#"(ns urls (:require [clojure.spec.alpha :as s]))
        (defn http? {:t/sig '[(s/nilable string?) :-> boolean?]} [^String url]
          (if url (.startsWith url "http") false))
        (defn suffix? {:t/sig '[string? :-> boolean?]} [text]
          (. text (endsWith ".")))
        (defn first-dot {:t/sig '[string? :-> number?]} [text]
          (.indexOf ^java.lang.String text "."))"#;
    let report = Session::default().check("urls.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn string_search_members_reject_wrong_and_nil_arguments() {
    let source = r#"(ns urls)
        (defn wrong-prefix [] (.startsWith "http" :http))
        (defn nil-suffix [] (.endsWith "text" nil))
        (defn wrong-needle [] (.indexOf "prefix." ["."]))"#;
    let report = Session::default().check("urls.clj", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .collect();
    assert_eq!(errors.len(), 3, "{report:#?}");
    for (error, (member, argument)) in errors.into_iter().zip([
        ("startsWith", ":http"),
        ("endsWith", "nil"),
        ("indexOf", r#"["."]"#),
    ]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            format!("java.lang.String/{member}")
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            argument
        );
    }
}

#[test]
fn receiver_hints_resolve_members_without_proving_a_non_nil_string() {
    let source = r#"(ns urls (:require [clojure.spec.alpha :as s]))
        (defn hinted-param {:t/sig '[(s/nilable string?) :-> boolean?]} [^String url]
          (.startsWith url "http"))
        (defn hinted-use {:t/sig '[(s/nilable string?) :-> number?]} [text]
          (.indexOf ^java.lang.String text "."))
        (defn nil-receiver {:t/sig '[:-> boolean?]} [] (.endsWith nil "."))"#;
    let report = Session::default().check("urls.clj", source);
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for (error, (member, actual, receiver)) in report.diagnostics.iter().zip([
        ("startsWith", "Nil | String", "url"),
        ("indexOf", "Nil | String", "text"),
        ("endsWith", "Nil", "nil"),
    ]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(error.expected.as_deref(), Some("String"));
        assert_eq!(error.actual.as_deref(), Some(actual));
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            format!("java.lang.String/{member}")
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            receiver
        );
    }
}

#[test]
fn string_search_reports_analysis_gaps_for_unknown_values_and_unmodeled_overloads() {
    let source = r#"(ns urls)
        (defn unknown-receiver {:t/sig '[map? :-> boolean?]} [m]
          (.startsWith (:url m) "http"))
        (defn hinted-unknown {:t/sig '[map? :-> boolean?]} [m]
          (let [^String url (:url m)] (.startsWith url "http")))
        (defn unknown-argument {:t/sig '[map? :-> boolean?]} [m]
          (.endsWith "text" (:suffix m)))
        (defn int-overload {:t/sig '[:-> number?]} [] (.indexOf "abc" 97))
        (defn offset-overload {:t/sig '[:-> boolean?]} [] (.startsWith "abc" "b" 1))
        (defn shadowed-hint {:t/sig '[string? :-> boolean?]} [^String url]
          (let [url {}] (.startsWith url "http")))
        (defn other-class {:t/sig '[map? :-> boolean?]} [m]
          (let [^java.nio.file.Path path (:path m)] (.startsWith path "dir")))"#;
    let report = Session::default().check("urls.clj", source);
    assert!(!report.complete, "{report:#?}");
    // The untyped map lookups also report their own missing type.
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code != "PNX9003")
        .collect();
    assert_eq!(gaps.len(), 7, "{report:#?}");
    for (gap, expression) in gaps.into_iter().zip([
        r#"(.startsWith (:url m) "http")"#,
        "url",
        "(:suffix m)",
        "97",
        r#"(.startsWith "abc" "b" 1)"#,
        r#"(.startsWith url "http")"#,
        r#"(.startsWith path "dir")"#,
    ]) {
        assert_eq!(gap.code, "PNX9001");
        assert_eq!(gap.severity, "warning");
        assert_eq!(
            &source[gap.span.start.offset..gap.span.end.offset],
            expression
        );
    }
}
