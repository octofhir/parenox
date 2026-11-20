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
             :note (str/trim (:value event))
             :counter (str (inc (:value event)))))
         (defn conjunction {:t/sig '[::event boolean? :-> string?]} [event flag]
           (if (and flag (= :note (:kind event)))
             (str/trim (:value event))
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
             (str/trim (:value event))
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
             (str/trim (:value event))
             (str/trim (:value event))))",
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
             :note (str/trim (:value event))))",
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
               (str/trim (:value event))
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
             (if (= kind :note) (str/trim (:value event)) \"\")))
         (defn alias-shadow {:t/sig '[::event :-> string?]} [event]
           (let [kind (:kind event) kind :note]
             (if (= kind :note) (str/trim (:value event)) \"\")))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    for diagnostic in &report.diagnostics {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.actual.as_deref(), Some("Number | String"));
    }
}

#[test]
fn copied_owners_and_projections_keep_the_original_binding_through_shadowing() {
    let report = check(
        "(s/def ::nullable (s/nilable string?))
         (defn nil-title {:t/sig '[nil? :-> string?]} [_] \"untitled\")
         (defn keep {:t/sig '[{:path ::nullable} {:path ::nullable} :-> string?]} [owner other]
           (let [saved owner path (:path owner) owner other]
             (if (nil? (:path saved)) (nil-title path) (subs path 0))))
         (defn self-shadow {:t/sig '[{:path ::nullable} :-> string?]} [owner]
           (let [saved owner owner (:path owner)]
             (if (nil? (:path saved)) (nil-title owner) (subs owner 0))))
         (defn scalar-copy {:t/sig '[::nullable :-> string?]} [path]
           (let [copy path]
             (if (nil? path) (nil-title copy) (subs copy 0))))
         (defn nested-tag {:t/sig '[::envelope :-> string?]} [message]
           (let [saved message payload (get-in saved [:item :value])]
             (case (get-in message [:item :kind])
               :note (str/trim payload)
               :counter (str (inc payload)))))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn presence_and_truthiness_refine_captured_fields_independently() {
    let report = check(
        "(s/def :clean/path string?)
         (s/def :flag/path (s/nilable (s/or :value string? :disabled #{false})))
         (s/def ::clean (s/keys :opt-un [:clean/path]))
         (s/def ::flag (s/keys :opt-un [:flag/path]))
         (defn nil-title {:t/sig '[nil? :-> string?]} [_] \"untitled\")
         (defn presence {:t/sig '[::clean :-> string?]} [owner]
           (let [saved owner path (:path saved)]
             (if (contains? owner :path) (subs path 0) (nil-title path))))
         (defn truthy {:t/sig '[::flag :-> string?]} [owner]
           (let [saved owner path (:path saved)]
             (if (:path owner) (subs path 0) \"untitled\")))
         (defn some-is-not-truthy {:t/sig '[::flag :-> string?]} [owner]
           (let [saved owner path (:path saved)]
             (if (some? (:path owner)) (subs path 0) \"untitled\")))
         (defn present-can-be-nil {:t/sig '[::flag :-> string?]} [owner]
           (let [saved owner path (:path saved)]
             (if (contains? owner :path) (subs path 0) (nil-title path))))",
    );
    let [falsey, nilable] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(falsey.actual.as_deref(), Some("false | String"));
    assert_eq!(nilable.actual.as_deref(), Some("Nil | false | String"));
}

#[test]
fn destructured_fields_share_the_saved_map_even_without_an_as_binding() {
    let report = check(
        "(defn parameter {:t/sig '[::event :-> string?]} [{:keys [kind value]}]
           (case kind :note (subs value 0) :counter (str (inc value))))
         (defn nested {:t/sig '[::envelope :-> string?]} [message]
           (let [{{:keys [value]} :item} message]
             (case (get-in message [:item :kind])
               :note (subs value 0) :counter (str (inc value)))))
         (defn shadowed-as {:t/sig '[:-> #{:field}]} []
           (let [{:keys [part] :as part} {:part :field}] part))
         (s/def ::label string?)
         (s/def ::box (s/keys :req-un [::item] :opt-un [::label]))
         (defn default-sibling {:t/sig '[::box :-> string?]} [message]
           (let [{:keys [item label] :or {label \"default\"} :as saved} message]
             (case (get-in saved [:item :kind])
               :note (str label (subs (:value item) 0))
               :counter (str label (inc (:value item))))))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn defaults_and_sequence_conversion_do_not_supply_a_map_selector() {
    let fallback = check(
        "(s/def ::path (s/nilable string?))
         (s/def ::ref (s/keys :opt-un [::path]))
         (defn render {:t/sig '[::ref :-> string?]} [ref]
           (let [{:keys [path] :or {path \"default\"} :as saved} ref]
             (if (some? path) (subs (:path saved) 0) \"empty\")))",
    );
    let [diagnostic] = fallback.diagnostics.as_slice() else {
        panic!("{fallback:#?}");
    };
    assert!(fallback.complete);
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | String"));

    let converted = check(
        "(defn from-pairs {:t/sig '[:-> map?]} []
           (let [{:as saved} (list :path \"ok\")] saved))",
    );
    let [diagnostic] = converted.diagnostics.as_slice() else {
        panic!("{converted:#?}");
    };
    assert!(!converted.complete);
    assert_eq!(diagnostic.code, "PNX9001");
    assert!(diagnostic.message.contains("sequence inputs"));
}

#[test]
fn defaults_shadowed_get_and_custom_predicates_do_not_prove_a_tag() {
    let report = check(
        "(s/def ::legacy (s/keys :opt-un [:counter/kind] :req-un [:counter/value]))
         (s/def ::with-legacy (s/or :note ::note :legacy ::legacy))
         (defn default-result {:t/sig '[::with-legacy :-> string?]} [event]
           (if (= (get event :kind :note) :note)
             (str/trim (:value event))
             \"other\"))
         (defn always-note {:t/sig '[::event keyword? :-> #{:note}]} [m k] :note)
         (defn shadow-get {:t/sig '[::event :-> string?]} [event]
           (let [get always-note]
             (if (= (get event :kind) :note) (str/trim (:value event)) \"\")))
         (defn accepts-tag {:t/sig '[keyword? :-> boolean?]} [kind] true)
         (defn no-assertion {:t/sig '[::event :-> string?]} [event]
           (if (accepts-tag (:kind event)) (str/trim (:value event)) \"\"))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3);
    for diagnostic in &report.diagnostics {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.actual.as_deref(), Some("Number | String"));
    }
}
