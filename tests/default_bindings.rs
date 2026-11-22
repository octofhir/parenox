use parenox::{Report, Session};

const SHORTHAND_FIXTURE: &str = include_str!("fixtures/map-binding-shorthands.clj");

fn check(body: &str) -> Report {
    let source = format!(
        r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(s/def ::name string?)
(s/def ::patient (s/keys :opt-un [::name]))
{body}
"#
    );
    Session::default().check("defaults.clj", &source)
}

#[test]
fn symbol_and_keyword_shorthand_bind_exact_scalar_keys() {
    let (source, _) = SHORTHAND_FIXTURE
        .split_once(";; Diagnostic cases.")
        .unwrap();
    let report = Session::default().check("shorthand.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn shorthand_reports_failures_at_the_operand_or_required_key() {
    let (source, _) = SHORTHAND_FIXTURE
        .split_once(";; JVM-only assertions.")
        .unwrap();
    let report = Session::default().check("shorthand.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let sites: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        sites,
        [
            ("PNX2322", "default"),
            ("PNX2741", "'cfg/required"),
            ("PNX2741", "early"),
            ("PNX2741", ":second"),
        ]
    );
}

#[test]
fn select_binds_the_projected_keys_and_preserves_nil_input() {
    let source = r#"
(ns demo.selected (:require [clojure.spec.alpha :as s]))
(defn options {:t/sig '[:-> (s/tuple number? #{true} nil? #{false})]} []
  (let [{:keys [item & :checked] :select picked :as original}
        {:item 7 :checked nil :extra false}]
    [(get picked :item) (contains? picked :checked) (get picked :extra) (get original :extra)]))
(defn symbol-fields {:t/sig '[:-> (s/tuple number? nil?)]} []
  (let [{:syms [item] :select picked} '{item 7 extra false}]
    [(get picked 'item) (get picked 'extra)]))
(defn empty-map {:t/sig '[:-> #{true}]} []
  (let [{:keys [item] :select picked} {}] (map? picked)))
(defn nil-input {:t/sig '[:-> nil?]} []
  (let [{:keys [item] :select picked} nil] picked))
"#;
    let report = Session::default().check("select.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn selected_binding_types_follow_edits_and_validate_default_keys() {
    let path = "select.clj";
    let source = r#"
(ns demo.selected)
(defn count-value {:t/sig '[:-> number?]} []
  (let [{:keys [value] :select picked} {:value 7}] (inc (:value picked))))
"#;
    let mut session = Session::default();
    let before = session.check(path, source);
    assert!(before.ok && before.complete, "{before:#?}");
    let changed = source.replace(":value 7", ":value \"text\"");
    let warm = session.check(path, &changed);
    let cold = Session::default().check(path, &changed);
    let [finding] = warm.diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &changed[finding.span.start.offset..finding.span.end.offset],
        "(:value picked)"
    );
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let defaults = source
        .replace(":select picked", ":or {value 2} :select picked")
        .replace("{:value 7}", "{}");
    let report = session.check(path, &defaults);
    assert!(report.ok && report.complete, "{report:#?}");

    let invalid = defaults
        .replace(":or {value 2}", ":or {unused 2}")
        .replace("(inc (:value picked))", "picked")
        .replace(":-> number?", ":-> any?");
    let report = session.check(path, &invalid);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete);
    assert_eq!(finding.code, "PNX1001");
    assert_eq!(
        &invalid[finding.span.start.offset..finding.span.end.offset],
        "unused"
    );
}

#[test]
fn missing_presence_only_key_stops_evaluation_before_the_binding_body() {
    let source = r#"
(ns demo.required)
(defn guarded {:t/sig '[:-> number?]} []
  (try
    (let [{:keys! [item & :missing]} {:item 1}]
      (inc "unreachable"))
    (catch IllegalArgumentException _ 7)))
"#;
    let report = Session::default().check("required.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2741");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        ":missing"
    );

    let resolved = source.replace("{:item 1}", "{:item 1 :missing nil}");
    let report = Session::default().check("required.clj", &resolved);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &resolved[finding.span.start.offset..finding.span.end.offset],
        "\"unreachable\""
    );

    let unresolved = source.replace(r#"(inc "unreachable")"#, "does-not-exist");
    let report = Session::default().check("required.clj", &unresolved);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2304")
    );
}

#[test]
fn excess_binds_unselected_fields_and_returns_nil_when_none_remain() {
    let source = r#"
(ns demo.excess (:require [clojure.spec.alpha :as s]))
(defn both {:t/sig '[:-> (s/tuple number? #{false} nil?)]} []
  (let [{:keys [keep & :checked] :select chosen :excess extras}
        {:keep 7 :checked nil :extra false}]
    [(get chosen :keep) (get extras :extra) (get extras :checked)]))
(defn finished {:t/sig '[:-> nil?]} []
  (let [{:syms [item] :excess extras} '{item 7}] extras))
(defn nil-input {:t/sig '[:-> nil?]} []
  (let [{:keys [item] :excess extras} nil] extras))
(defn defaulted {:t/sig '[:-> (s/tuple #{:default} number?)]} []
  (let [{:keys [keep] :or {keep :default} :excess extras} {:extra 7}]
    [keep (get extras :extra)]))
(defn nullable {:t/sig '[boolean? :-> (s/nilable {:extra nil?})]} [with-extras]
  (let [input (if with-extras {:keep 7 :extra nil} {:keep 7})
        {:keys [keep] :excess extras} input]
    extras))
"#;
    let report = Session::default().check("excess.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn open_nested_excess_binds_an_unknown_local_and_reports_the_coverage_gap() {
    let source = r#"
(ns demo.excess)
(defn nested {:t/sig '[{:nested map?} :-> any?]} [row]
  (let [{{} :nested :excess extras} row]
    extras))
"#;
    let report = Session::default().check("excess.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "extras"
    );
}

#[test]
fn missing_keys_preserve_success_and_follow_warm_edits() {
    let source = r#"
(ns demo.missing)
(defn value {:t/sig '[:-> number?]} []
  (let [{:keys! [item] :missing absent} {:item 7}]
    (inc item)))
"#;
    let path = "missing.clj";
    let mut session = Session::default();
    let good = session.check(path, source);
    assert!(good.ok && good.complete, "{good:#?}");

    let edited = source.replace("{:item 7}", "{}");
    let warm = session.check(path, &edited);
    let cold = Session::default().check(path, &edited);
    assert_eq!(warm.complete, cold.complete);
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let [error] = warm.diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(!warm.ok && warm.complete);
    assert_eq!(error.code, "PNX2322");
    assert_eq!(
        &edited[error.span.start.offset..error.span.end.offset],
        "item"
    );

    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(
        serde_json::to_value(&restored.diagnostics).unwrap(),
        serde_json::to_value(&good.diagnostics).unwrap()
    );
}

#[test]
fn nested_defaults_can_see_the_initial_missing_binding() {
    let source = r#"
(defn f {:t/sig '[:-> (s/tuple nil? #{true})]} []
  (let [{{:keys [name] :or {name absent}} :inner
         {:keys! [id]} :another
         :missing absent}
        {}]
    [name (contains? (:another absent) :id)]))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn defaults_handle_absence_and_preserve_nil_false_and_precise_values() {
    let report = Session::default().check("defaults.clj", include_str!("../examples/defaults.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {name "Unknown"}} nil] name))"#,
        r#"(defn f {:t/sig '[::patient :-> string?]} [{:keys [name] :or {:name "Unknown"}}] name)"#,
        r#"(defn f {:t/sig '[:-> boolean?]} [] (let [{:keys [active] :or {active true}} {:active false}] active))"#,
        r#"(s/def ::status #{"active"}) (defn f {:t/sig '[:-> ::status]} [] (let [{:keys [status] :or {status "active"}} {}] status))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {name "Unknown"}} {:name nil}] name))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
    let report =
        check(r#"(defn f {:t/sig '[::patient :-> number?]} [{:keys [name] :or {name 1}}] name)"#);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
}

#[test]
fn default_values_use_the_reader_maps_evaluation_order() {
    let function = r#"
(defn f {:t/sig '[:-> string?]} []
  (let [{:keys [key0 key1 key2 key3 key4 key5 key6 key7 key8]
         :or {:key0 (throw nil) :key1 1 :key2 2 :key3 (inc "hash-order")
              :key4 4 :key5 5 :key6 6 :key7 7 TAIL 8}} {}]
    key0))
"#;
    let ordered = check(&function.replace("TAIL", ":key8"));
    assert!(ordered.ok && ordered.complete, "{ordered:#?}");
    let hashed = check(&function.replace("TAIL", r#""key8""#));
    let [error] = hashed.diagnostics.as_slice() else {
        panic!("{hashed:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("String"));
}

#[test]
fn string_key_defaults_distinguish_explicit_nil_from_absence() {
    let source = include_str!("../examples/json_fields.cljc");
    let mut session = Session::default();
    let checked = session.check("json_fields.cljc", source);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    let edit = source.replace("(or name \"Unknown\")", "name");
    let bad = session.check("json_fields.cljc", &edit);
    let [error] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));

    let result = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:strs [name] :or {:name 1 "name" "default"}} {}] name))"#,
    );
    assert!(result.ok && result.complete, "{result:#?}");
}

#[test]
fn defaults_are_eager_and_share_outer_scope_instead_of_new_map_locals() {
    let report = Session::default().check("bad.clj", include_str!("../examples/defaults_bad.clj"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        ["PNX2322", "PNX2322", "PNX1001"]
    );
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [x "outer" {:keys [x y] :or {x 1 y x}} {}] y))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report =
        check("(defn f {:t/sig '[:-> number?]} [] (let [{:keys [x y] :or {x 1 y x}} {}] y))");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2304"),
        "{report:#?}"
    );
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {unused (inc "bad")}} {:name "Ada"}] name))"#,
    );
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2322"),
        "{report:#?}"
    );
}

#[test]
fn renamed_namespaced_and_nested_bindings_use_local_or_lookup_key_defaults() {
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{n :name :or {n "Unknown"}} {}] n))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{n :name :or {:name "Unknown"}} {}] n))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:person/keys [name] :or {:person/name "Unknown"}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [person/name] :or {name "Unknown"}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{{:keys [name] :or {name "Unknown"}} :patient :or {:patient {}}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:req! {{:keys [name] :or {name "Unknown"}} :patient}} {:patient nil}] name))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [person/name] :or {person/name "Unknown"}} {}] name))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
}

#[test]
fn ambiguous_or_required_defaults_are_invalid_and_point_at_the_default_key() {
    for pattern in [
        r#"{:keys [name] :or {name "A" :name "B"}}"#,
        r#"{:keys [name] :or {name "A" :name "B"} :select picked :all merged}"#,
        r#"{:keys! [name] :or {name "A"}}"#,
        r#"{:req! {n :name} :or {:name "A"}}"#,
        r#"{:keys! [id & :name] :or {:name "A"}}"#,
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> string?]}} [] (let [{pattern} {{:id 1 :name \"Ada\"}}] \"ok\"))"
        );
        let report = Session::default().check("bad.clj", &source);
        assert!(!report.ok && report.complete, "{pattern}: {report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.code, "PNX1001");
        assert!(matches!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            "name" | ":name"
        ));
    }
    let report =
        check("(defn f {:t/sig '[:-> number?]} [] (let [{:keys [id] :or []} {:id 1}] id))");
    assert_eq!(report.diagnostics[0].code, "PNX1001");
}

#[test]
fn parameter_defaults_see_ordinary_arguments_and_prior_destructured_parameters() {
    for body in [
        r#"(defn f {:t/sig '[::patient string? :-> string?]} [{:keys [name] :or {name fallback}} fallback] name)"#,
        r#"(defn f {:t/sig '[::patient ::patient :-> string?]} [{x :name :or {x "first"}} {:keys [name] :or {name x}}] name)"#,
        r#"(defn- label [{:keys [name] :or {name fallback}} fallback] name) (defn f {:t/sig '[:-> string?]} [] (label {} "Unknown"))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn helper_edits_in_parameter_defaults_invalidate_cached_callers() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(s/def ::name string?) (s/def ::patient (s/keys :opt-un [::name]))
(defn f {:t/sig '[::patient :-> string?]} [{:keys [name] :or {name (fallback)}}] name)
(defn- fallback [] "X")
"#;
    let mut session = Session::default();
    let report = session.check("cache.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(session.check("cache.clj", source).stats.reused_functions, 1);
    let changed = source.replace("\"X\"", "nil");
    let report = session.check("cache.clj", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}

#[test]
fn missing_projection_detects_a_shadowed_accumulator() {
    let source = r#"
        (defn f {:t/sig '[:-> any?]} []
          (let [{:keys! [absent] :missing absent} {}] absent))
    "#;
    let report = Session::default().check("shadowed.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(gap.code, "PNX9001");
    assert!(gap.message.contains("also bound"));
    assert_eq!(
        &source[gap.span.start.offset..gap.span.end.offset],
        "absent"
    );

    let source = r#"
        (defn f {:t/sig '[:-> #{true}]} []
          (let [{:keys! [name] :as absent :missing absent} {}]
            (contains? absent :name)))
    "#;
    let report = Session::default().check("alias.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn deeply_nested_missing_bindings_complete() {
    let mut pattern = "{:keys! [leaf]}".to_owned();
    for _ in 0..95 {
        pattern = format!("{{{pattern} :child}}");
    }
    let source = format!(
        "(defn probe {{:t/sig '[:-> map?]}} [] (let [{{{pattern} :root :missing absent}} {{}}] absent))"
    );
    let report = Session::default().check("deep.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_rebound_missing_child_keeps_the_proof_of_its_current_value() {
    let source = r#"
        (defn f {:t/sig '[{1 #{:hit}} :-> #{:hit}]} [row]
          (let [{{:keys! [x] :missing collected} :inside
                 collected :replacement
                 :missing absent}
                {:replacement row}]
            (get-in absent [:inside 1N] :missing)))
    "#;
    let report = Session::default().check("rebound.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete);
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some(":hit | :missing"));
}

#[test]
fn an_open_nested_selection_reports_the_gap_at_its_named_output() {
    let source = r#"
        (defn f {:t/sig '[{:inside map?} :-> any?]} [row]
          (let [{{} :inside :select picked} row] picked))
    "#;
    let report = Session::default().check("open.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(
        &source[gap.span.start.offset..gap.span.end.offset],
        "picked"
    );
}

#[test]
fn finite_map_destructuring_of_keyword_arguments_preserves_exact_operand_ranges() {
    let path = "sequence.clj";
    let source = r#"
(ns demo.sequence)
(defn next-count {:t/sig '[:-> number?]} []
  (let [{:keys [count] :all opts} (list :count 7 {})]
    (inc (:count opts))))
"#;
    let mut session = Session::default();
    let before = session.check(path, source);
    assert!(before.ok && before.complete, "{before:#?}");
    let changed = source.replace(":count 7", ":count \"text\"");
    let warm = session.check(path, &changed);
    let cold = Session::default().check(path, &changed);
    let [finding] = warm.diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(!warm.ok && warm.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &changed[finding.span.start.offset..finding.span.end.offset],
        "(:count opts)"
    );
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn unproved_keyword_argument_inputs_report_coverage() {
    let source = r#"
(ns demo.sequence (:require [clojure.spec.alpha :as s]))
(defn large {:t/sig '[:-> any?]} []
  (let [{:as options} (list 1N :first 1 :later)] options))
(defn suffix {:t/sig '[(s/coll-of any? :kind vector?) :-> any?]} [tail]
  (let [{:as options} (list :item 7 tail)] options))
"#;
    let report = Session::default().check("unproved.clj", source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    let sites: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(sites, [("PNX9001", "{:as options}"); 2], "{report:#?}");
}

#[test]
fn alternating_map_inputs_report_an_operand_mismatch_and_keep_open_sequences_unchecked() {
    let source = r#"
(ns demo.alternatives (:require [clojure.spec.alpha :as s]))
(defn incompatible {:t/sig '[boolean? :-> number?]} [choose?]
  (let [{:keys [count]} (if choose? (list :count "text") {:count 7})]
    (inc count)))
(defn open-tail {:t/sig '[boolean? (s/coll-of keyword? :kind list?) :-> any?]}
  [choose? arguments]
  (let [{:as options} (if choose? (list :count 7) arguments)] options))
"#;
    let report = Session::default().check("alternatives.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert!(!report.ok && !report.complete);
    assert_eq!(
        findings,
        [("PNX2322", "count"), ("PNX9001", "{:as options}")],
        "{report:#?}"
    );
}
