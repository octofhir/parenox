use parenox::{Report, Session};

const EXAMPLE: &str = include_str!("../examples/tagged_maps.clj");

fn check(extra: &str) -> Report {
    Session::default().check("tagged.clj", &format!("{EXAMPLE}\n{extra}"))
}

#[test]
fn equality_and_case_narrow_the_owner_and_nested_aliases() {
    let report = check(
        "(defn exhaustive {:t/sig '[::event :-> string?]} [event]
           (case (get event :kind)
             :note (str/upper-case (:value event))
             :counter (str (inc (:value event)))))
         (defn conjunction {:t/sig '[::event boolean? :-> string?]} [event flag]
           (if (and flag (= :note (:kind event)))
             (str/upper-case (:value event))
             \"fallback\"))
         (defn contradictory {:t/sig '[::event :-> number?]} [event]
           (let [kind (:kind event)]
             (if (and (= :note (:kind event)) (= kind :counter))
               (inc \"unreachable\")
               0)))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_false_conjunction_does_not_prove_the_opposite_tag() {
    let report = check(
        "(defn ambiguous {:t/sig '[::event boolean? :-> string?]} [event flag]
           (if (and flag (= :note (:kind event)))
             (str/upper-case (:value event))
             (str (inc (:value event)))))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
}

#[test]
fn a_known_tag_still_rejects_the_wrong_payload_operation() {
    let report = check(
        "(defn broken {:t/sig '[::event :-> string?]} [event]
           (if (= (get event :kind) :note)
             (str/upper-case (:value event))
             (str/upper-case (:value event))))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "PNX2322");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Number"));
}

#[test]
fn partial_case_reports_the_unhandled_known_tag() {
    let report = check(
        "(defn missing {:t/sig '[::event :-> string?]} [event]
           (case (:kind event)
             :note (str/upper-case (:value event))))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "PNX2802");
    assert!(report.diagnostics[0].message.contains(":counter"));
}

#[test]
fn missing_nil_and_false_have_different_guard_behavior() {
    let report = check(
        "(s/def :legacy/kind #{nil false})
         (s/def :legacy/value number?)
         (s/def ::legacy (s/keys :opt-un [:legacy/kind] :req-un [:legacy/value]))
         (s/def ::with-legacy (s/or :note ::note :legacy ::legacy))
         (defn with-legacy {:t/sig '[::with-legacy :-> string?]} [event]
           (if (some? (:kind event))
             (if (:kind event)
               (str/upper-case (:value event))
               (str (inc (:value event))))
             (str (inc (:value event)))))
         (s/def :text/name string?)
         (s/def ::optional (s/keys :opt-un [:text/name]))
         (defn require-present {:t/sig '[::optional :-> string?]} [m]
           (if (nil? (get m :name))
             (get m :name \"fallback\")
             (req! m :name)))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn shadowing_the_owner_or_alias_drops_the_old_relationship() {
    let report = check(
        "(defn owner-shadow {:t/sig '[::event ::event :-> string?]} [event replacement]
           (let [kind (:kind event) event replacement]
             (if (= kind :note) (str/upper-case (:value event)) \"\")))
         (defn alias-shadow {:t/sig '[::event :-> string?]} [event]
           (let [kind (:kind event) kind :note]
             (if (= kind :note) (str/upper-case (:value event)) \"\")))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    for diagnostic in &report.diagnostics {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.actual.as_deref(), Some("Number | String"));
    }
}

#[test]
fn defaults_shadowed_get_and_custom_predicates_do_not_prove_a_tag() {
    let report = check(
        "(s/def ::legacy (s/keys :opt-un [:counter/kind] :req-un [:counter/value]))
         (s/def ::with-legacy (s/or :note ::note :legacy ::legacy))
         (defn default-result {:t/sig '[::with-legacy :-> string?]} [event]
           (if (= (get event :kind :note) :note)
             (str/upper-case (:value event))
             \"other\"))
         (defn always-note {:t/sig '[::event keyword? :-> #{:note}]} [m k] :note)
         (defn shadow-get {:t/sig '[::event :-> string?]} [event]
           (let [get always-note]
             (if (= (get event :kind) :note) (str/upper-case (:value event)) \"\")))
         (defn accepts-tag {:t/sig '[keyword? :-> boolean?]} [kind] true)
         (defn no-assertion {:t/sig '[::event :-> string?]} [event]
           (if (accepts-tag (:kind event)) (str/upper-case (:value event)) \"\"))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3);
    for diagnostic in &report.diagnostics {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.actual.as_deref(), Some("Number | String"));
    }
}
