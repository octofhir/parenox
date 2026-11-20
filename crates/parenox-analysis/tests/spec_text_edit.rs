use parenox_analysis::Session;

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-text-edit.clj");

#[test]
fn conforms_text_edits_in_known_lists_and_sequences() {
    let specs = FIXTURE.split_once(";; JVM-only").unwrap().0;
    let cases = r#"
(def edit {:new-text "🧪"
           :range {:start {:line 1N :character 2} :end {:line 3 :character 4}}})
(def wide {:new-text "wide"
           :range {:start {:line 2147483648 :character 2} :end {:line 3 :character 4}}})

(defn direct {:t/sig '[:-> int?]} []
  (get-in (s/conform ::text-edit edit) [:range :start :line]))
(defn direct-invalid {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::text-edit (dissoc wide :new-text)))
(defn normalized-list {:t/sig '[:-> (s/coll-of map? :kind list?)]} []
  (s/conform ::text-edits (list edit edit)))
(defn normalized-seq {:t/sig '[:-> (s/coll-of map? :kind list?)]} []
  (s/conform ::edits (seq [edit])))
(defn second-tag {:t/sig '[:-> #{:second}]} []
  (let [edits (list (assoc edit :tag :first) (assoc edit :tag :second))]
    (:tag (first (rest (s/conform ::additional-text-edits edits))))))
(defn nested-line {:t/sig '[:-> int?]} []
  (-> (s/conform ::text-edits (list edit)) first :range :start :line))
(defn empty-list {:t/sig '[:-> (s/coll-of string? :kind list?)]} []
  (s/conform ::text-edits '()))
(defn empty-set {:t/sig '[:-> (s/coll-of string? :kind set?)]} []
  (s/conform ::text-edits #{}))
(defn invalid-first {:t/sig '[:-> #{::s/invalid}]} []
  (try
    (s/conform ::text-edits (list nil wide))
    (catch ArithmeticException _ :unreachable)))
(defn overflow-first {:t/sig '[:-> #{:overflow}]} []
  (try
    (s/conform ::text-edits (seq [wide nil]))
    (catch ArithmeticException _ :overflow)))
"#;
    let report = Session::default().check("text_edit.clj", &format!("{specs}{cases}"));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn conforms_hash_set_literals_without_assuming_member_order() {
    let specs = FIXTURE.split_once(";; JVM-only").unwrap().0;
    let cases = r#"
(def edit {:new-text "🧪"
           :range {:start {:line 1N :character 2} :end {:line 3 :character 4}}})
(def wide {:new-text "wide"
           :range {:start {:line 2147483648 :character 2} :end {:line 3 :character 4}}})
(defn normalized-set {:t/sig '[:-> (s/coll-of ::text-edit :kind set?)]} []
  (s/conform ::text-edits #{edit (assoc edit :tag :other)}))
(defn winner {:t/sig '[:-> (s/nilable int?)]} []
  (when-let [item (first (s/conform ::text-edits #{edit (assoc edit :tag :other)}))]
    (get-in item [:range :start :line])))
(defn invalid {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::additional-text-edits #{edit nil}))
(defn overflow {:t/sig '[:-> #{:overflow}]} []
  (try
    (s/conform ::edits #{wide})
    (catch ArithmeticException _ :overflow)))
(defn either {:t/sig '[int? :-> #{::s/invalid :overflow}]} [tag]
  (let [invalid (assoc edit :new-text nil :tag tag)]
    (try
      (s/conform ::text-edits #{invalid wide})
      (catch ArithmeticException _ :overflow))))
"#;
    let check = |cases| Session::default().check("text_edit.clj", &format!("{specs}{cases}"));
    let report = check(cases);
    assert!(report.ok && report.complete, "{report:#?}");

    let only_invalid = cases.replace("#{::s/invalid :overflow}", "#{::s/invalid}");
    let only_overflow = cases.replace("#{::s/invalid :overflow}", "#{:overflow}");
    for (body, missing) in [
        (&only_invalid, ":overflow"),
        (&only_overflow, "clojure.spec.alpha/invalid"),
    ] {
        let report = check(body);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX2322", "{diagnostic:#?}");
        assert!(diagnostic.message.contains(missing), "{diagnostic:#?}");
    }
}
