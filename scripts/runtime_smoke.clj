;; Optional independent smoke check against actual Clojure/JVM.
;; Run from the project root: clojure -M scripts/runtime_smoke.clj
(require '[clojure.edn :as edn])
(let [publics (ns-publics 'clojure.core)
      groups (group-by (comp boolean :macro meta val) publics)
      names-in (fn [macro?] (sort (map (comp str key) (get groups macro?))))
      catalog (edn/read-string (slurp "crates/parenox-analysis/src/core_names/jvm.edn"))]
  (assert (= {:macros (names-in true) :vars (names-in false)} catalog)))
(println "Clojure/JVM public core catalog: 1 assertion passed")

(let [scope (create-ns 'parenox.smoke.default-imports)
      imports (ns-imports scope)
      catalog (edn/read-string
               (slurp "crates/parenox-analysis/src/checker/model/imports/jvm.edn"))
      expected (into {} (map (fn [^String name]
                               [(symbol (subs name (inc (.lastIndexOf name ".")))) name]))
                     catalog)
      actual (into {} (map (fn [[name ^Class type]] [name (.getName type)])) imports)]
  (assert (= expected actual))
  (assert (not (contains? imports 'Record))))
(println "Clojure/JVM default class import catalog: 2 assertions passed")

(load-file "examples/bad.clj")
(assert (= :throws
           (try (demo/display-name {:id "1"})
                :unexpected-success
                (catch NullPointerException _ :throws))))
(load-file "examples/good.clj")
(assert (= "UNKNOWN" (demo/display-name {:id "1"})))
(assert (= "ALICE" (demo/display-name {:id "1" :name "Alice"})))
(assert (= "Unknown" (demo/guarded-name {:id "1"})))
(assert (= "ALICE" (demo/guarded-name {:id "1" :name "Alice"})))
(assert (= 3 (demo/call-increment 2)))
(assert (= nil (get {:name nil} :name "fallback")))
(assert (= "fallback" (get {} :name "fallback")))
(assert (= 1 (if 0 1 2)))
(assert (= "value" (or false nil "value")))
(println "Clojure/JVM smoke: 10 assertions passed")

;; A set remains a Clojure set in value position; membership returns a value or nil.
(load-file "examples/sets.clj")
(assert (set? (demo.sets/statuses)))
(assert (= "active" (demo.sets/default-status)))
(assert (= 2 (demo.sets/status-count)))
(assert (= "active" (demo.sets/find-status "active")))
(assert (nil? (demo.sets/find-status "deleted")))
(assert (= :active (get #{:active} :active)))
(assert (= 1 (if #{} 1 2)))
(require '[clojure.set :as set])
(assert (= [true false false]
           (mapv #(int? (get % 1N))
                 [(set/union #{1N} #{1 :extra})
                  (set/union #{1N :extra} #{1})
                  (set/union #{1N} #{1})])))
(println "Clojure/JVM sets: 8 assertions passed")

(load-file "examples/clinic/model.clj")
(load-file "examples/clinic/store.clj")
(load-string (clojure.string/replace
               (slurp "examples/clinic/service.clj")
               "(str/upper-case (:name patient))"
               "(str/upper-case (or (:name patient) \"Unknown\"))"))
(load-string (-> (slurp "examples/clinic/http.clj")
                 (clojure.string/replace "(store/find-patient 42)" "(store/find-patient id)")
                 (clojure.string/replace "(str/starts-with? 42" "(str/starts-with? id")))
(assert (= {:status 200 :body "ALICE"} (clinic.http/show-patient {:path-params {:id "p-1"}})))
(assert (= 404 (:status (clinic.http/show-patient {:path-params {:id "p-2"}}))))
(assert (= 400 (:status (clinic.http/show-patient {:path-params {:id "invalid"}}))))
(assert (= "UNKNOWN" (clinic.service/display-name {:id "p-2"})))
(println "Clojure/JVM clinic: 4 assertions passed")

(load-file "examples/inferred_bad.clj")
(assert (= :throws
           (try (demo.inferred/display-name {})
                :unexpected-success
                (catch NullPointerException _ :throws))))
(load-file "examples/inferred.clj")
(assert (= "UNKNOWN" (demo.inferred/display-name {})))
(assert (= "ALICE" (demo.inferred/display-name {:name "Alice"})))
(println "Clojure/JVM private helpers: 3 assertions passed")

(load-file "examples/reader.clj")
(assert (= "UNKNOWN " (demo.reader/label nil \space)))
(assert (= "ALICE-" (demo.reader/label "Alice" (demo.reader/default-separator))))
(assert (true? (:private (meta #'demo.reader/normalize))))
(assert (= false (:private (meta (read-string "^{:private false} ^:private item")))))
(assert (= true (:private (meta (read-string "^:private ^{:private false} item")))))
(assert (= [\] \a \Ω] (read-string "[\\] \\o141 \\u03A9]")))
(assert (= 'map' (read-string "map'")))
(assert (= 42 ((eval '(fn [^String x] x)) 42)))
(println "Clojure/JVM metadata and characters: 8 assertions passed")

(doseq [read [read-string edn/read-string]]
  (assert (= [0xd83d 0xde03 33]
             (mapv int (read "\"\\uD83D\\uDE03!\""))))
  (assert (= [97 0xd800 0xd83d 0xde03 10]
             (mapv int (read "\"a\\uD800😃\\n\""))))
  (assert (= [Double/POSITIVE_INFINITY Double/NEGATIVE_INFINITY]
             (read "[##Inf ##-Inf]")))
  (assert (Double/isNaN (read "## ; comment\n #_discard NaN"))))
(println "Clojure/JVM and EDN strings and symbolic numbers: 8 assertions passed")

(defmacro reader-template [value options] `(let [item# ~value] [item# ~@options]))
(assert (= [:first :second :third] (reader-template :first [:second :third])))
(assert (try (read-string "`~@items") false (catch RuntimeException _ true)))
(println "Clojure/JVM syntax quote: 2 assertions passed")

(load-file "examples/jvm_forms.clj")
(let [box (demo.jvm-forms/->TextBox "text")]
  (assert (= "text" (.text ^demo.jvm_forms.TextSource box)))
  (assert (= :ready (demo.jvm-forms/ready box))))
(assert (= "Untitled" (demo.jvm-forms/display-name {:name nil})))
(assert (= "1:Hello" (demo.jvm-forms/render "Hello")))
(assert (= :absent (demo.jvm-forms/ready nil)))
(println "Clojure/JVM declarations: 5 assertions passed")

(require '[clojure.spec.alpha :as s])
(let [label (s/with-gen string? #(throw (Exception. "generator ran")))
      catalog (s/map-of keyword? label :gen-max 1)
      labels (s/coll-of label :gen-max 1 :kind vector?)]
  (assert (s/valid? catalog {:a "Ada" :b "Grace"}))
  (assert (s/valid? labels ["Ada" "Grace"]))
  (assert (not (s/valid? catalog {:a nil}))))
(println "Clojure/JVM spec generator settings: 3 assertions passed")

(s/def :parenox.smoke/set-predicate #{nil false true :yes ::s/invalid})
(assert (= [false false true true false]
           (mapv #(s/valid? :parenox.smoke/set-predicate %) [nil false true :yes ::s/invalid])))
(assert (= [::s/invalid :yes]
           (mapv #(s/conform :parenox.smoke/set-predicate %) [false :yes])))
(assert (= [true true false]
           (mapv #(s/valid? (s/nilable false?) %) [nil false (Boolean. false)])))
(println "Clojure/JVM set predicates: 3 assertions passed")

(println "Runtime reference:" (clojure-version))
(load-file "examples/portable.cljc")
(assert (= "ALICE" (demo.portable/label "Alice")))
(assert (= [\space \-] (demo.portable/separators)))
(assert (nil? (ns-resolve 'demo.portable 'browser-label)))
(let [read-clj #(read-string {:read-cond :allow} %)]
  (assert (= :fallback (read-clj "#?(:default :fallback :clj :jvm)")))
  (assert (= [] (read-clj "[#?(:cljs :absent)]")))
  (assert (= [:a :b] (read-clj "[#?@(:clj (:a :b))]")))
  (assert (= {:a 1} (read-clj "{#?@(:clj [:a 1])}")))
  (assert (= '(quote :next) (read-clj "'#?(:cljs :absent) :next")))
  (assert (= :default (read-clj "#?(:clj #?(:cljs :absent) :default :fallback)")))
  (assert (= :jvm (read-clj "#?(:cljs #?(:clj :nested) :clj :jvm)")))
  (assert (try (read-clj "#?(:cljs #?(:other 1) :clj 2)") false (catch RuntimeException _ true)))
  (assert (try (read-clj "#?(:clj 1 :cljs #?(:else 2))") false (catch RuntimeException _ true))))
(println "Clojure/JVM reader conditionals: 12 assertions passed")

(load-file "examples/checked.clj")
(load-file "examples/checked_bad.clj")
(assert (= "ADA" (demo.checked/display-name {:id "p-1" :name "Ada"})))
(assert (= "p-1: Unknown" (demo.checked/label {:id "p-1"})))
(assert (= "p-1" (demo.checked/request-id {:patient {:id "p-1"}})))
(assert (= "Missing required key: :name"
           (try (demo.checked/display-name {:id "p-1"}) nil
                (catch RuntimeException e (.getMessage e)))))
(assert (= "Missing required key: :name"
           (try (demo.checked-bad/missing-name) nil
                (catch RuntimeException e (.getMessage e)))))
(assert (= :throws
           (try (demo.checked-bad/nil-name) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (nil? (req! {:name nil} :name)))
(assert (false? (req! {:active false} :active)))
(assert (= :active (req! #{:active} :active)))
(assert (= "Ada" (let [{:person/keys! [name & :id]} {:person/name "Ada" :person/id nil}] name)))
(assert (= "Ada" (let [{:keys! [person/name]} {:person/name "Ada"}] name)))
;; This changes throwing semantics and remains explicitly unsupported by Parenox.
(assert (= [nil {:name nil}] (let [{:req! {n :name} :missing absent} {}] [n absent])))
(println "Clojure/JVM checked bindings: 12 assertions passed")

(load-file "examples/defaults.clj")
(assert (= "UNKNOWN" (demo.defaults/display-name {:id "p-1"})))
(assert (= "ADA" (demo.defaults/display-name {:id "p-1" :name "Ada"})))
(assert (= "Configured" (demo.defaults/configured-name {:id "p-1"} "Configured")))
(assert (= "Unknown" (demo.defaults/inferred-name {:id "p-1"})))
;; The last definition is intentionally invalid at macroexpansion time.
;; Earlier definitions remain available to verify their independent runtime errors.
(assert (= :invalid
           (try (load-file "examples/defaults_bad.clj") :unexpected-success
                (catch clojure.lang.Compiler$CompilerException _ :invalid))))
(assert (= :throws
           (try (demo.defaults-bad/nil-name) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (demo.defaults-bad/invalid-fallback) :unexpected-success
                (catch ClassCastException _ :throws))))
(assert (= "outer" (let [x "outer" {:keys [x y] :or {x 1 y x}} {}] y)))
(assert (= "Unknown" (let [{:person/keys [name] :or {:person/name "Unknown"}} {}] name)))
(assert (= "Unknown" (let [{:req! {{:keys [name] :or {name "Unknown"}} :patient}} {:patient nil}] name)))
(assert (nil? (let [{:keys [person/name] :or {person/name "Unknown"}} {}] name)))
(assert (= :throws
           (try (let [{:keys [name] :or {unused (inc "bad")}} {:name "Ada"}] name)
                :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM destructuring defaults: 12 assertions passed")
(load-file "tests/fixtures/map-binding-shorthands.clj")

(load-file "examples/threading.clj")
(load-file "examples/threading_bad.clj")
(assert (= "UNKNOWN" (demo.threading/display-name {})))
(assert (= "ADA" (demo.threading/display-name {:name " Ada "})))
(assert (= "7: Ada" (demo.threading/numbered-name "Ada")))
(assert (nil? (demo.threading/maybe-name nil)))
(assert (= "ADA" (demo.threading/maybe-name " Ada ")))
(assert (nil? (demo.threading/maybe-numbered nil)))
(assert (= "7: Ada" (demo.threading/maybe-numbered "Ada")))
(assert (= "false" (demo.threading/false-text)))
(assert (nil? (some-> nil (str (inc "bad")))))
(assert (= "seed" (->> "seed" (let [{:keys [user/parenox__thread_0]} {:user/parenox__thread_0 42}]))))
(assert (= [3 1] (let [calls (atom 0) result (some-> (swap! calls inc) inc inc)] [result @calls])))
(assert (= :throws
           (try (demo.threading-bad/unsafe-name {}) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (demo.threading-bad/bad-number) :unexpected-success
                (catch ClassCastException _ :throws))))
(assert (nil? (demo.threading-bad/optional-result nil)))
(println "Clojure/JVM threading macros: 14 assertions passed")

(load-file "examples/conditional.clj")
(load-file "examples/conditional_bad.clj")
(assert (= "ADA!" (demo.conditional/display-name " Ada " true)))
(assert (= "PATIENT: ADA" (demo.conditional/prefixed-name "Ada" true)))
(assert (= "ADA" (demo.conditional/prefixed-name "Ada" false)))
(assert (= "UNKNOWN" (demo.conditional/defaulted-name nil)))
(assert (= "ADA" (demo.conditional/defaulted-name "Ada")))
(assert (= "7: ADA" (demo.conditional/flexible-name " Ada " 7)))
(assert (= "AB" (demo.conditional/after-nil)))
(assert (= "" (as-> nil value (str value) nil (str value))))
(assert (= :name (as-> {} value :name)))
(assert (= ["B" [:seed :a :b]]
           (let [events (atom [])
                 result (cond-> (do (swap! events conj :seed) nil)
                          (do (swap! events conj :a) false) (str "A")
                          (do (swap! events conj :b) true) (str "B"))]
             [result @events])))
(assert (= 42 (let [value 42 answer (as-> "seed" value (str value))] value)))
(assert (= "1" (demo.conditional-bad/mixed-result true)))
(assert (= :throws
           (try (demo.conditional-bad/unsafe-name nil) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (demo.conditional-bad/bad-number "Ada") :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM conditional and named threading: 14 assertions passed")

(load-file "examples/guards.clj")
(assert (= "UNKNOWN" (demo.guards/display-name {})))
(assert (= "ADA" (demo.guards/maybe-label {:name "Ada"})))
(assert (= "inactive" (demo.guards/status false)))
(assert (= "unknown" (demo.guards/status nil)))
(assert (= "inactive" (demo.guards/maybe-status false)))
(assert (nil? (demo.guards/maybe-status nil)))
(assert (= [:outer 1]
           (let [value :outer calls (atom 0)
                 result (if-let [value (do (swap! calls inc) false)] value value)]
             [result @calls])))
(println "Clojure/JVM binding guards: 7 assertions passed")

(load-file "examples/paths.clj")
(load-file "examples/paths_bad.clj")
(assert (= "UNKNOWN" (demo.paths/display-name {})))
(assert (= "ADA" (demo.paths/display-name {:patient {:name "Ada"}})))
(assert (= "Ada" (demo.paths/normalized-name {} " Ada ")))
(assert (nil? (get-in {:patient {:name nil}} [:patient :name] "Unknown")))
(assert (false? (get-in {:patient {:active false}} [:patient :active] true)))
(assert (= "Unknown" (get-in {:patient 42} [:patient :name] "Unknown")))
(assert (= {:patient {:id 42 :name "Ada"}}
           (assoc-in {:patient {:id 42}} [:patient :name] "Ada")))
(assert (= :throws
           (try (demo.paths-bad/wrong-parent) :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM nested paths: 8 assertions passed")

(load-file "examples/presence.clj")
(load-file "examples/presence_bad.clj")
(assert (= "ADA" (demo.presence/display-name {:name "Ada"})))
(assert (= "UNKNOWN" (demo.presence/display-name nil)))
(assert (= "inactive" (demo.presence/status {:active false})))
(assert (= "unknown" (demo.presence/status {})))
(assert (contains? {:name nil} :name))
(assert (not (contains? [:name] :name)))
(assert (contains? #{:name} :name))
(assert (= :throws
           (try (demo.presence-bad/nullable-name {:name nil}) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= "Missing required key: :name"
           (try (demo.presence-bad/missing-name {}) :unexpected-success
                (catch RuntimeException e (.getMessage e)))))
(println "Clojure/JVM map presence: 9 assertions passed")

(load-file "examples/updates.clj")
(load-file "examples/updates_bad.clj")
(assert (= 5 (demo.updates/visits-after {:visits 3} 2)))
(assert (= 2 (demo.updates/visits-after {} 2)))
(assert (= "UNKNOWN" (demo.updates/display-name {})))
(assert (= "ADA" (demo.updates/display-name {:patient {:name "Ada"}})))
(assert (= {:x false} (update {:x false} :x identity)))
(assert (= {:x {:y nil}} (update-in {:x nil} [:x :y] identity)))
(assert (= :throws
           (try (demo.updates-bad/increment-visits {}) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (demo.updates-bad/normalize-name) :unexpected-success
                (catch NullPointerException _ :throws))))
(println "Clojure/JVM map updates: 8 assertions passed")

(assert (= {:visits 1} (update {} :visits (fnil inc 0))))
(assert (= {:p {:x 3}} (update-in {} [:p :x] (fnil + 0) 3)))
(assert (= false ((fnil identity true) false)))
(assert (= 3 ((fnil + 1 2 100) nil nil)))
(assert (= 103 ((fnil + 1 2 100) nil nil nil)))
(println "Clojure/JVM fnil: 5 assertions passed")

(assert (= 5 (let [delta 2] (:visits (update {:visits 3} :visits (fn [old] (+ old delta)))))))
(assert (= 9 ((fn [{:keys [x] :or {x y}} y] x) {} 9)))
(assert (= 2 ((fn [x x] x) "first" 2)))
(assert (nil? ((fn [x]) 42)))
(assert (= 1 ((fnil (fn [n] (inc n)) 0) nil)))
(println "Clojure/JVM inline fn: 5 assertions passed")

(load-file "examples/vectors.clj")
(load-file "examples/vectors_bad.clj")
(assert (= ["Patient ADA" "Patient LIN"]
           (demo.vectors/display-labels [{:name "Ada"} {} {:name nil} {:name "Lin"}])))
(assert (= [10 23] (demo.vectors/adjust-counts [nil 3 4] [10 20])))
(assert (= [] (mapv inc [] [1])))
(assert (= [] (mapv inc nil)))
(assert (= [\a \b] (mapv identity "ab")))
(assert (= [false 0 ""] (filterv some? [nil false 0 ""])))
(assert (= [0 ""] (filterv identity [nil false 0 ""])))
(assert (= :throws
           (try (demo.vectors-bad/increment-counts) :unexpected-success
                (catch ClassCastException _ :throws))))
(assert (= :throws
           (try (demo.vectors-bad/uppercase-names [nil]) :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (mapv (fnil inc (inc "bad")) []) :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM vector callbacks: 10 assertions passed")

(load-file "examples/entries.clj")
(load-file "examples/entries_bad.clj")
(assert (= [":patient ADA"] (demo.entries/display-labels "Ada" "")))
(assert (= [:patient] (demo.entries/recorded-roles "Ada")))
(assert (= [nil] (mapv val {:admitted nil})))
(assert (= [] (mapv val {})))
(assert (= [[:admitted 12 nil 2 :missing false]]
           (mapv (fn [[k v extra :as entry]]
                   [k v extra (count entry) (get entry :other :missing)
                    (contains? entry :admitted)])
                 {:admitted 12})))
(assert (= :throws
           (try (demo.entries-bad/increment-counts) :unexpected-success
                (catch ClassCastException _ :throws))))
(assert (= :throws
           (try (demo.entries-bad/key-of-pair) :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM map entries: 7 assertions passed")

(assert (= "Unlabeled" (demo.entries/preferred-label {})))
(assert (nil? (demo.entries/preferred-label {:label nil})))
(assert (= "Ada" (demo.entries/preferred-label {:label "Ada"})))
(assert (map-entry? (find {:enabled false} :enabled)))
(assert (false? (val (find {:enabled false} :enabled))))
(assert (= [0 nil] (find [nil false] 0)))
(assert (nil? (find [nil false] -1)))
(println "Clojure/JVM entry lookup: 7 assertions passed")

(load-file "examples/dictionaries.clj")
(load-file "examples/dictionaries_bad.clj")
(assert (clojure.spec.alpha/valid? :demo.dictionaries/prices {}))
(assert (= ["SKU-1: 12"] (demo.dictionaries/price-labels {"sku-1" 12})))
(assert (= 1 (demo.dictionaries/next-open-count {})))
(assert (= 5 (demo.dictionaries/next-open-count {:open 4})))
(assert (= {:open 1 :closed 3} (demo.dictionaries/record-open {:closed 3})))
(assert (= {:open 5 :closed 3} (demo.dictionaries/record-open {:open 4 :closed 3})))
(assert (= nil (get {:open nil} :open 0)))
(assert (not (clojure.spec.alpha/valid?
               :demo.dictionaries/counts
               (demo.dictionaries-bad/record-open {:closed 3}))))
(assert (= :throws
           (try (demo.dictionaries-bad/increment-sku {"sku-1" 12}) :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM dictionaries: 9 assertions passed")

(load-file "examples/map_keys.clj")
(load-file "examples/map_keys_bad.clj")
(assert (= 140 (demo.map-keys/quoted-price {"SKU-17" 140} "SKU-17" 100)))
(assert (= 100 (demo.map-keys/quoted-price {} "SKU-17" 100)))
(assert (= 110 (demo.map-keys/discounted-price {"SKU-17" 140} "SKU-17" 30)))
(assert (= 0 (demo.map-keys/discounted-price {} "SKU-17" 30)))
(assert (= {"SKU-17" 120 "SKU-18" 50}
           (demo.map-keys/adjust-price {"SKU-17" 100 "SKU-18" 50} "SKU-17" 20)))
(assert (= {"SKU-17" 20} (demo.map-keys/adjust-price {} "SKU-17" 20)))
(assert (= 120 (demo.map-keys/catalog-price)))
(assert (= [1 "two"] [(get {:x 1 "x" "two"} :x) (get {:x 1 "x" "two"} "x")]))
(assert (nil? (get {"SKU-17" nil} "SKU-17" 120)))
(assert (= :throws
           (try (demo.map-keys-bad/next-price {"SKU-17" nil} "SKU-17")
                :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (not (clojure.spec.alpha/valid?
               :demo.map-keys/prices
               (demo.map-keys-bad/mark-pending {"SKU-17" 120} "SKU-17"))))
(println "Clojure/JVM map keys: 11 assertions passed")

(load-file "examples/map_projection.clj")
(assert (= {:id "p-1" :enabled false "display" nil}
           (demo.map-projection/public-profile
             {:id "p-1" :enabled false "display" nil :internal-token "internal" :extra 1})))
(assert (= {:id "p-2" :enabled true}
           (demo.map-projection/public-profile {:id "p-2" :enabled true :internal-token "internal"})))
(assert (= {} (select-keys nil [:missing]) (select-keys {:id 42} nil)))
(println "Clojure/JVM map projection: 3 assertions passed")

(assert (= {:id "p-1" "internal-token" false :other nil}
           (demo.map-projection/redact-profile
             {:id "p-1" :internal-token "internal" "internal-token" false :other nil})))
(assert (= {"id" nil} (dissoc {:id 42 "id" nil} :id)))
(assert (= 42 (dissoc 42)))
(assert (nil? (dissoc nil :x 42)))
(assert (= :class-cast
           (try (dissoc 42 :id)
                (catch ClassCastException _ :class-cast))))
(println "Clojure/JVM map removal: 5 assertions passed")

(load-file "examples/map_sequences.clj")
(assert (= 3 (demo.map-sequences/total-quota {:alpha 1 :beta 2})))
(assert (= 0 (demo.map-sequences/total-quota {})))
(assert (nil? (demo.map-sequences/first-service {})))
(let [row (array-map :beta 2 :alpha 1)]
  (assert (= '(:beta :alpha) (keys row)))
  (assert (= '(2 1) (vals row))))
(assert (= '(nil false) (vals (array-map :missing nil :disabled false))))
(assert (= '(:entry) (keys (seq {:entry 1}))))
(println "Clojure/JVM map keys and values: 7 assertions passed")

(load-file "examples/update_vals.clj")
(assert (= {:daily "UNTITLED" :weekly "REPORT"}
           (demo.update-vals/normalize-labels {:daily nil :weekly "report"})))
(assert (= {:count 2} (demo.update-vals/advance {:count 1})))
(assert (= {:count 5 :label "label"} (demo.update-vals/advance {:count 4 :label " label "})))
(assert (= {} (update-vals nil 42)))
(assert (= {:a true :b false} (update-vals {:a nil :b false} nil?)))
(assert (reduced? (:value (update-vals {:value 1} reduced))))
(assert (= :caught
           (try (update-vals {:value nil} (fn [_] (throw (ex-info "failed" {}))))
                (catch clojure.lang.ExceptionInfo _ :caught))))
(println "Clojure/JVM map value transforms: 7 assertions passed")

(load-file "examples/map_builders.clj")
(assert (= {"amount" 5 "unit" "meters"} (demo.map-builders/public-reading [5 "meters"])))
(assert (= {:a 2} (demo.map-builders/totals [:a :a :extra] [1 2])))
(assert (= {"done" false} (demo.map-builders/statuses {:done false})))
(assert (= {:a false :b nil} (zipmap [:a :b :a :unused] [1 nil false])))
(assert (= {} (zipmap [:unused] [])))
(assert (= {} (update-keys nil 42)))
(assert (= {"new" nil} (update-keys {:old nil} (fn [_] "new"))))
(println "Clojure/JVM map builders: 7 assertions passed")

(load-file "examples/group_by.clj")
(assert (= {:queued [{:status :queued :id 2} {:status :queued :id 3}]
            :done [{:status :done :id 1}]}
           (demo.group-by/tasks-by-status [{:status :queued :id 2}
                                           {:status :done :id 1}
                                           {:status :queued :id 3}])))
(assert (= {:text ["first" "last"] :flag [false]} (demo.group-by/labels-by-kind)))
(assert (= {"state" [nil false]} (group-by (fn [_] "state") [nil false])))
(assert (= {} (group-by 42 nil)))
(println "Clojure/JVM grouping: 4 assertions passed")

(load-file "examples/merge_with.clj")
(assert (= {:cpu 5 :disk 8}
           (demo.merge-with/sum-quotas {:cpu 2} {:cpu 3 :disk 8})))
(assert (= {:id "p-1" :label "first / second" :private false}
           (demo.merge-with/join-labels {:id "p-1" :label "first" :private false}
                                       {:label "second"})))
(assert (= {:id "p-1" :label "first"}
           (demo.merge-with/join-labels {:id "p-1" :label "first"} {})))
(assert (nil? (merge-with 42 nil nil)))
(assert (= {:a nil :b false} (merge-with 42 {:a nil} {:b false})))
(assert (= {:a [[nil false] 1]} (merge-with vector {:a nil} {:a false} {:a 1})))
(println "Clojure/JVM merge callbacks: 6 assertions passed")

(load-file "examples/dynamic_paths.clj")
(load-file "examples/dynamic_paths_bad.clj")
(assert (= 140 (demo.dynamic-paths/quote-price {"SKU-17" {:retail 140}} "SKU-17" :retail 100)))
(assert (= 100 (demo.dynamic-paths/quote-price {} "SKU-17" :retail 100)))
(assert (= {"SKU-17" {:retail 140 :wholesale 80}}
           (demo.dynamic-paths/reprice {"SKU-17" {:wholesale 80}} "SKU-17" 140)))
(assert (= {"SKU-17" {:retail 20}} (demo.dynamic-paths/adjust-price {} "SKU-17" 20)))
(assert (= {"SKU-17" {:price 141 :stock 3}}
           (demo.dynamic-paths/raise-current-price {"SKU-17" {:price 140 :stock 3}} "SKU-17")))
(assert (= {} (demo.dynamic-paths/raise-current-price {} "SKU-17")))
(assert (nil? (let [sku "SKU-17" tier :retail]
                (get-in {"SKU-17" {:retail nil}} [sku tier] 100))))
(assert (= :throws
           (try (demo.dynamic-paths-bad/raise-price {} "SKU-17") :unexpected-success
                (catch NullPointerException _ :throws))))
(assert (= :throws
           (try (demo.dynamic-paths-bad/add-retail-tier {"SKU-17" 140} "SKU-17") :unexpected-success
                (catch ClassCastException _ :throws))))
(println "Clojure/JVM dynamic paths: 9 assertions passed")

(load-file "examples/ordered_paths.clj")
(load-file "examples/ordered_paths_bad.clj")
(assert (= 140 (demo.ordered-paths/retail-quote {"SKU-17" {:retail 140}} "SKU-17")))
(assert (= 100 (demo.ordered-paths/quote-price {} ["SKU-17" :wholesale] 100)))
(assert (= {"SKU-17" {:retail 150 :wholesale 80} "SKU-18" {:retail 25}}
           (demo.ordered-paths/adjust-both-prices
             {"SKU-17" {:retail 140} "SKU-18" {:retail 25}} "SKU-17" 80 10)))
(assert (= "SKU-17: 140" (demo.ordered-paths/price-label ["SKU-17" 140])))
(assert (clojure.spec.alpha/valid? :demo.ordered-paths/route ["SKU-17" :retail]))
(assert (not (clojure.spec.alpha/valid? :demo.ordered-paths/route [:retail "SKU-17"])))
(assert (= 2 (let [path (if false [:east :retail] [:west :wholesale])]
               (get-in {:east {:retail 1} :west {:wholesale 2}} path))))
(assert (= [:retail 140 nil] (let [[k v missing] [:retail 140]] [k v missing])))
(assert (= :throws
           (try (demo.ordered-paths-bad/increment-quantity) :unexpected-success
                (catch NullPointerException _ :throws))))
(println "Clojure/JVM ordered paths: 9 assertions passed")

(load-file "examples/indices.clj")
(load-file "examples/indices_bad.clj")
(assert (= 120 (demo.indices/row-price ["SKU-17" 120])))
(assert (= 120 (demo.indices/first-item-price {:items [{:price 120}]} 90)))
(assert (= 90 (demo.indices/first-item-price {:items []} 90)))
(assert (= 0 (demo.indices/first-rate [])))
(assert (= 120 (demo.indices/first-rate [120 90])))
(assert (= :fallback (nth [:sku] -1 :fallback)))
(assert (= nil (nth nil 8)))
(assert (contains? [:sku nil] 1))
(assert (= 1 (nth [1 2] 0.5)))
(assert (= :fallback (get [1 2] 0.5 :fallback)))
(assert (= :throws
           (try (demo.indices-bad/beyond-row) :unexpected-success
                (catch IndexOutOfBoundsException _ :throws))))
(assert (= :throws
           (try (demo.indices-bad/increment-explicit-nil) :unexpected-success
                (catch NullPointerException _ :throws))))
(println "Clojure/JVM indexed reads: 12 assertions passed")

(let [scope (create-ns 'parenox.smoke.partial-bounds)]
  (binding [*ns* scope]
    (clojure.core/refer 'clojure.core :rename '{nth at subvec slice})
    (doseq [form '[(at [1] 2) (let [_ "length"] (at [1] 2))
                   (slice [1] 2) (let [_ "subs"] (slice [1] 2))]]
      (assert (try (eval form) false (catch IndexOutOfBoundsException _ true))))
    (assert (= 2 (eval '(let [at (fn [values index] index)] (at [1] 2)))))))
(println "Clojure/JVM resolved bounds operations: 5 assertions passed")

(load-file "tests/fixtures/index-key-classes.clj")
(load-file "tests/fixtures/numeric-map-key-classes.clj")

(load-file "examples/loops.clj")
(assert (= 10 (demo.loops/triangular 5)))
(assert (= "Ada" (demo.loops/carried-label "Ada")))
(assert (= 15 (demo.loops/nested-total 5)))
(assert (= 2 (demo.loops/last-value)))
(assert (= 7 (demo.loops/local-function)))
(assert (= 2 (let [y 1] (loop [{:keys [x] :or {x y}} {} y 2] x))))
(assert (= "changed" (loop [again? true value 0]
                       (if again? (recur false "changed") value))))
(let [forms (with-open [reader (java.io.PushbackReader. (clojure.java.io/reader "examples/loops_bad.clj"))]
              (loop [forms []]
                (let [form (read {:eof ::eof} reader)]
                  (if (= form ::eof) forms (recur (conj forms form))))))]
  (binding [*ns* *ns*]
    (eval (first forms))
    (eval (second forms))
    (assert (= :throws (try ((ns-resolve *ns* 'invalid-later))
                            (catch ClassCastException _ :throws))))
    (doseq [[form expected] (map vector (drop 2 forms)
                                ["Mismatched argument count to recur, expected: 2"
                                 "Can only recur from tail position"
                                 "Mismatched argument count to recur, expected: 1"])]
      (assert (try (eval form) false
                   (catch clojure.lang.Compiler$CompilerException e
                     (clojure.string/includes? (ex-message (ex-cause e)) expected)))))))
(println "Clojure/JVM loops: 11 assertions passed")

(load-file "examples/cond.clj")
(load-file "examples/cond_bad.clj")
(assert (= ["MISSING" "ADA" "3"] (mapv demo.cond/label [nil "Ada" 2])))
(assert (= "Ada" (demo.cond/loop-label " Ada ")))
(assert (nil? (demo.cond-bad/missing-label nil)))
(assert (= 3 (demo.cond-bad/increment-wrong-branch 2)))
(assert (= :throws (try (demo.cond-bad/increment-wrong-branch "Ada")
                        (catch ClassCastException _ :throws))))
(assert (= 5 (cond 0 5 :else (inc "bad"))))
(assert (nil? (cond)))
(println "Clojure/JVM cond: 7 assertions passed")

(load-file "examples/libspecs/provider.clj")
(load-file "examples/libspecs/app.clj")
(load-file "examples/libspecs/bad.clj")
(assert (= 5 (demo.libspecs.app/bump-counter 2)))
(assert (= "Ada" (demo.libspecs.app/clean-label " Ada ")))
(assert (nil? (ns-resolve 'demo.libspecs.app 'advance)))
(assert (nil? (ns-resolve 'demo.libspecs.app 'spare)))
(assert (= :throws (try (demo.libspecs.bad/broken-counter)
                        (catch ClassCastException _ :throws))))
(println "Clojure/JVM libspecs: 5 assertions passed")

(load-file "examples/namespace_specs/config.clj")
(assert (not (contains? (loaded-libs) 'demo.namespace-specs.handler)))
(assert (= :demo.namespace-specs.handler/request-id
           (demo.namespace-specs.config/request-key)))
(load-file "examples/namespace_specs/handler.clj")
(load-file "examples/namespace_specs/app.clj")
(load-file "examples/namespace_specs/bad.clj")
(assert (= :demo.namespace-specs.handler/request-id
           (demo.namespace-specs.handler/header)))
(assert (clojure.spec.alpha/valid? :wire/id (demo.namespace-specs.app/sample-id)))
(assert (not (clojure.spec.alpha/valid? :wire/id (demo.namespace-specs.bad/sample-id))))
(println "Clojure/JVM namespace specs: 5 assertions passed")

(load-file "examples/case.clj")
(load-file "examples/case_bad.clj")
(assert (= ["pending" "pending" "finished"]
           (mapv demo.case/format-status [:queued :running :done])))
(assert (= [1 0] (mapv demo.case/flag-code [true false])))
(assert (= ["zero" "one" "other"] (mapv demo.case/digit-label [0 1 2])))
(assert (= "Ada" (demo.case/published-label "Ada")))
(assert (= :done (loop [x "Aa"] (case x "Aa" (recur "BB") "BB" :done))))
(assert (= :match (case :a () missing :a :match :default)))
(assert (= :throws (try (demo.case-bad/bad-flag-code true)
                        (catch ClassCastException _ :throws))))
(assert (= :throws (try (demo.case-bad/absent-status)
                        (catch IllegalArgumentException _ :throws))))
(println "Clojure/JVM case: 8 assertions passed")

(load-file "examples/sequences.clj")
(load-file "examples/sequences_bad.clj")
(assert (= 10 (demo.sequences/sum-amounts [1 2 3 4])))
(assert (= 0 (demo.sequences/sum-amounts [])))
(assert (= "second" (demo.sequences/second-label)))
(assert (= "label" (demo.sequences/positional-label)))
(assert (nil? (seq [])))
(assert (seq [nil]))
(assert (nil? (first (seq [nil 1]))))
(assert (= "next" (first (next [1 "next"]))))
(assert (= "next" (nth (seq [1 "next"]) 1)))
(assert (= :throws (try (demo.sequences-bad/increment-head [])
                        (catch NullPointerException _ :throws))))
(assert (= :throws (try (demo.sequences-bad/start-with-false)
                        (catch IllegalArgumentException _ :throws))))
(println "Clojure/JVM sequences: 11 assertions passed")

(load-file "examples/accumulators.clj")
(load-file "examples/accumulators_bad.clj")
(assert (= ["ADA" "UNKNOWN" "LIN"] (demo.accumulators/labels ["Ada" nil "Lin"])))
(assert (= (vec (repeat 100 "ADA")) (demo.accumulators/labels (vec (repeat 100 "Ada")))))
(assert (= [] (demo.accumulators/labels [])))
(assert (= [[1 2] ["1" "2"]] (demo.accumulators/project-values [1 2])))
(assert (= [42] (demo.accumulators/collect-once)))
(assert (= [] (conj)))
(assert (= "same" (conj "same")))
(assert (= [1 "second"] (conj [1] "second")))
(assert (= '("second" 1) (conj nil 1 "second")))
(assert (= #{:ready :done} (conj #{:ready} :done)))
(assert (= [true false false nil false true false]
           [(int? (get (conj #{1} 1N) 1N))
            (int? (get (conj #{1N} 1) 1))
            (int? (get (conj #{} 1N 1) 1))
            (get (conj #{:other} nil) nil :miss)
            (get (conj #{:other} false) false :miss)
            (contains? (conj #{:initial} :added) :added)
            (contains? (conj #{:initial} :added) :missing)]))
(assert (= :throws (try (demo.accumulators-bad/unchecked-labels [nil])
                        (catch NullPointerException _ :throws))))
(let [items (disj #{1 :keep false nil} 1N nil)]
  (assert (= [false :keep false nil :same nil nil :keep]
             [(contains? items 1) (items :keep) (get items false :wrong) (get items nil)
              (disj :same) (disj nil :a) (disj false :a)
              (get (apply disj [#{1 :keep} 1N]) :keep)])))
(let [items (set [nil :item false :item])]
  (assert (= [true false nil false true false false true]
             [(contains? items :item) (contains? items :missing)
              (get items nil :missing) (get items false :missing)
              (contains? (set '(:x :x :y)) :y) (contains? (set nil) :x)
              (int? (get (set [1 1N]) 1))
              (int? (first (set [1 1N])))])))
(assert (= [:head true false true nil :present nil false]
           [(first (set [:head :head]))
            (= 1 (count (set [:head :head])))
            (empty? (conj #{} :head))
            (contains? (not-empty (set [:head])) :head)
            (not-empty (disj #{1} 1N))
            (if (seq (set [:head])) :present :empty)
            (next (set [:head :head]))
            (contains? (set (rest (set [:head]))) :head)]))
(println "Clojure/JVM accumulators: 15 assertions passed")

(load-file "examples/exceptions.clj")
(load-file "examples/exceptions_bad.clj")
(assert (= "req-1" (demo.exceptions/required-id "req-1")))
(assert (= {:field :id :code "MISSING"}
           (try (demo.exceptions/required-id nil)
                (catch clojure.lang.ExceptionInfo e (ex-data e)))))
(assert (= "MISSING" (demo.exceptions/error-code)))
(assert (= "unknown" (demo.exceptions/absent-code)))
(assert (= {} (ex-data (ex-info "No details" nil))))
(assert (nil? (ex-data 1)))
(assert (= "Stopped"
           (try (demo.exceptions/stop-before-later-arguments)
                (catch clojure.lang.ExceptionInfo e (ex-message e)))))
(assert (= :throws (try (throw nil) (catch NullPointerException _ :throws))))
(assert (= :throws (try (demo.exceptions-bad/bad-exception)
                        (catch ClassCastException _ :throws))))
(assert (= :throws (try (demo.exceptions-bad/bad-cause)
                        (catch ClassCastException _ :throws))))
(println "Clojure/JVM exceptions: 10 assertions passed")

(load-file "examples/handlers.clj")
(assert (= "req-1" (demo.handlers/request-id "req-1")))
(assert (= "unknown-1" (demo.handlers/request-id nil)))
(assert (= 10 (demo.handlers/guarded-count 5)))
(assert (= "Cleanup failed"
           (try (demo.handlers/cleanup-fails)
                (catch ExceptionInfo e (ex-message e)))))
(assert (= 1 (try (throw nil)
                  (catch RuntimeException e 1)
                  (catch NullPointerException e "unreachable"))))
(assert (nil? (try (throw nil) (catch Throwable e))))
(assert (= :invalid
           (try (load-file "examples/handlers_bad.clj")
                (catch clojure.lang.Compiler$CompilerException e
                  (when (= "Cannot recur across try" (ex-message (ex-cause e))) :invalid)))))
(assert (= :throws
           (try ((ns-resolve 'demo.handlers-bad 'invalid-recovery))
                (catch ClassCastException _ :throws))))
(assert (= "MISSING" (demo.handlers/request-code nil)))
(assert (= "req-1" (demo.handlers/request-code "req-1")))
(assert (= "CLEANUP" (demo.handlers/cleanup-code)))
(println "Clojure/JVM handlers: 11 assertions passed")

(load-file "examples/literal_vars/config.clj")
(load-file "examples/literal_vars/app.clj")
(assert (= 5 (demo.literal-vars.app/next-limit)))
(assert (= :demo.literal-vars.config/online (demo.literal-vars.app/plan-mode)))
(assert (= :active (demo.literal-vars.app/status-code)))
(assert (= "Unknown" (demo.literal-vars.config/fallback-name)))
(assert (= :not-public
           (try (load-file "examples/literal_vars/bad.clj")
                (catch clojure.lang.Compiler$CompilerException e
                  (when (clojure.string/includes? (ex-message (ex-cause e)) "is not public")
                    :not-public)))))
(println "Clojure/JVM literal vars: 5 assertions passed")

(load-file "examples/core_imports.clj")
(assert (= 4 (demo.core-imports/label-width "Ada")))
(assert (= 4 (demo.core-imports/prior 5)))
(assert (nil? (ns-resolve 'demo.core-imports 'dec)))
(assert (= :unbound (try (load-file "examples/core_imports_bad.clj")
                         (catch clojure.lang.Compiler$CompilerException e
                           (when (clojure.string/includes? (ex-message (ex-cause e))
                                                          "Unable to resolve symbol: inc")
                             :unbound)))))
(println "Clojure/JVM core imports: 4 assertions passed")

(assert (= [4 5] (demo.vectors/increment-after-first [1 nil 3 4])))
(assert (= [] (demo.vectors/increment-after-first [1])))
(assert (= [] (demo.vectors/increment-after-first [])))
(println "Clojure/JVM vector callbacks over seqs: 3 assertions passed")

(load-file "examples/arities/provider.clj")
(load-file "examples/arities/app.clj")
(load-file "examples/arities/bad.clj")
(assert (= 10 (demo.arities.app/next-offset 9)))
(assert (= 9 (demo.arities.provider/count-offset 9 :done)))
(assert (= 3 (demo.arities.app/label-size "Ada")))
(assert (= "ready" (demo.arities.app/ready-label)))
(assert (= :wrong-arity (try (demo.arities.bad/too-many)
                            (catch clojure.lang.ArityException _ :wrong-arity))))
(assert (= :wrong-phase (try (demo.arities.bad/wrong-phase)
                            (catch IllegalArgumentException _ :wrong-phase))))
(println "Clojure/JVM fixed arities: 6 assertions passed")

(assert (= [3 2] (demo.arities.app/label-widths ["Ada" "Bo"])))
(assert (= [] (demo.arities.app/label-widths [])))
(assert (= 2 ((fn ([x] x) ([x keep?] (if keep? (recur x false) x))) 2 true)))
(println "Clojure/JVM inline fixed arities: 3 assertions passed")

(load-file "examples/variadic.clj")
(load-file "examples/variadic_bad.clj")
(assert (= 4 (demo.variadic/adjust 4)))
(assert (= 7 (demo.variadic/adjust 4 3)))
(assert (= 7 (demo.variadic/inline-offset)))
(assert (nil? (demo.variadic/first-label false)))
(assert (= "next" (demo.variadic/first-label true "skip" "next")))
(assert (= :wrong-arity (try (demo.variadic-bad/missing-base)
                            (catch clojure.lang.ArityException _ :wrong-arity))))
(assert (= :wrong-element (try (demo.variadic-bad/wrong-element)
                               (catch ClassCastException _ :wrong-element))))
(println "Clojure/JVM variadic arities: 7 assertions passed")

(load-file "examples/apply.clj")
(load-file "examples/apply_bad.clj")
(assert (= "ready" (demo.apply/fallback)))
(assert (= 7 (demo.apply/offset)))
(assert (= "queue/ready" (demo.apply/join-args)))
(assert (= [4 7] (mapv demo.apply/maybe-offset [false true])))
(assert (= :wrong-element (try (demo.apply-bad/wrong-element)
                               (catch ClassCastException _ :wrong-element))))
(assert (= :wrong-count (try (demo.apply-bad/wrong-count)
                             (catch clojure.lang.ArityException _ :wrong-count))))
(assert (= :target (try
                     (apply (throw (ex-info "target" {:which :target}))
                            (throw (ex-info "prefix" {:which :prefix}))
                            (throw (ex-info "spread" {:which :spread})))
                     (catch clojure.lang.ExceptionInfo e (:which (ex-data e))))))
(println "Clojure/JVM apply: 7 assertions passed")

(load-file "examples/anonymous.clj")
(load-file "examples/anonymous_bad.clj")
(assert (= [2 8] (demo.anonymous/offsets [1 4])))
(assert (= "Ada:3" (demo.anonymous/label-id 3 "Ada")))
(assert (= "ready" (demo.anonymous/first-extra)))
(assert (= "settled" (demo.anonymous/settle)))
(assert (= :wrong-element (try (demo.anonymous-bad/increment-labels ["Ada"])
                               (catch ClassCastException _ :wrong-element))))
(assert (= :wrong-arity (try (demo.anonymous-bad/missing-slot)
                             (catch clojure.lang.ArityException _ :wrong-arity))))
(assert (= "shadow" (#(let [% "shadow"] %) 1)))
(assert (= 6 (#(inc #_ %2 %) 5 :ignored)))
(assert (= 5 ((eval (read-string {:read-cond :allow}
                                  "#(identity #?(:clj % :cljs %3))"))
              5 :second :third)))
(doseq [bad ["#(inc %0)" "#(inc %01)" "#(str %x)" "#(str #_ #(str %) %)"]]
  (assert (= :reader-error (try (read-string bad)
                                (catch IllegalStateException _ :reader-error)))))
(println "Clojure/JVM shorthand callbacks: 13 assertions passed")

(assert (= "Untitled" (demo.threading/choose-label true)))
(assert (= :input (try (demo.threading/choose-label false)
                        (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= 2 (-> [n 1] (let (inc n)))))
(assert (= :done (loop [again false] (->> (recur true) (if again :done)))))
(assert (= :done (loop [again true] (if again (-> (recur false) do) :done))))
(assert (= :outer
           (try
             (->> (throw (ex-info "seed" {:stage :seed}))
                  (str (throw (ex-info "outer" {:stage :outer}))))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :seed
           (try
             (some->> (throw (ex-info "seed" {:stage :seed})) (if true :body))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(println "Clojure/JVM threading order: 7 assertions passed")

(assert (= 1 (demo.apply/increment-default)))
(assert (= :default
           (try
             ((fnil identity (throw (ex-info "default" {:stage :default})))
              (throw (ex-info "argument" {:stage :argument})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :callee
           (try
             ((fnil (throw (ex-info "callee" {:stage :callee}))
                    (throw (ex-info "default" {:stage :default}))) nil)
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= 6 (apply (fnil (fnil + 1 2) nil 5) [nil nil])))
(println "Clojure/JVM fnil evaluation: 4 assertions passed")

(assert (= :collection (demo.vectors/collection-stage)))
(assert (= :callback
           (try
             (mapv (fnil identity (throw (ex-info "callback" {:stage :callback})))
                   (throw (ex-info "collection" {:stage :collection})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :first
           (try
             (mapv + (throw (ex-info "first" {:stage :first}))
                     (throw (ex-info "second" {:stage :second})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(println "Clojure/JVM vector argument evaluation: 3 assertions passed")

(assert (= :updater (demo.updates/updater-stage)))
(assert (= :target
           (try
             (update (throw (ex-info "target" {:stage :target}))
                     (throw (ex-info "key" {:stage :key}))
                     (fnil + (throw (ex-info "updater" {:stage :updater}))))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :path
           (try
             (update-in {} [:nested (throw (ex-info "path" {:stage :path}))]
                            (fnil + (throw (ex-info "updater" {:stage :updater}))))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :inside
           (try
             (update {} :visits (fn [_] (throw (ex-info "inside" {:stage :inside}))))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(assert (= :extra
           (try
             (update-in {} [:visits] (fn [_ delta] delta)
                            (throw (ex-info "extra" {:stage :extra})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))))
(println "Clojure/JVM update argument evaluation: 5 assertions passed")

(assert (= 4 (demo.apply/total-offset 4 [])))
(assert (= 10 (demo.apply/total-offset 4 [1 2 3])))
(assert (= 780 (demo.apply/total-offset 0 (vec (range 40)))))
(assert (= :wrong-count (try (apply inc (next [1 2 3]))
                             (catch clojure.lang.ArityException _ :wrong-count))))
(println "Clojure/JVM open argument spreads: 4 assertions passed")

(assert (= 0 (demo.sequences/accumulate-with-rest [])))
(assert (= 12 (demo.sequences/accumulate-with-rest [3 4 5])))
(assert (= () (rest nil)))
(assert (= () (rest [nil])))
(assert (if (rest [1]) true false))
(assert (nil? (seq (rest [1]))))
(assert (nil? (first (rest [1 nil]))))
(assert (= "tail" (nth (rest [1 "tail"]) 0)))
(assert (= "missing" (nth (rest [1]) 0 "missing")))
(assert (= 0 (apply + (rest [4]))))
(assert (= "reset" (first (conj (rest [4]) "reset"))))
(println "Clojure/JVM rest and empty lists: 11 assertions passed")

(load-file "examples/all_refers/library.clj")
(load-file "examples/all_refers/app.clj")
(assert (= "ready" (demo.all-refers.app/render)))
(assert (= 17 (demo.all-refers.app/window-size)))
(assert (= 4 (demo.all-refers.app/fallback-size)))
(assert (nil? (ns-resolve 'demo.all-refers.app 'label)))
(assert (nil? (ns-resolve 'demo.all-refers.app 'skipped)))
(assert (nil? (ns-resolve 'demo.all-refers.app 'hidden)))
(assert (= :excluded
           (try
             (load-file "examples/all_refers/bad.clj")
             (catch clojure.lang.Compiler$CompilerException e
               (when (clojure.string/includes? (ex-message (ex-cause e))
                                               "Unable to resolve symbol: skipped")
                 :excluded)))))
(println "Clojure/JVM all referrals: 7 assertions passed")

(assert (= :private
           (try
             (load-string "(demo.all-refers.library/hidden)")
             (catch clojure.lang.Compiler$CompilerException e
               (when (clojure.string/includes? (ex-message (ex-cause e)) "is not public")
                 :private)))))
(println "Clojure/JVM source visibility: 1 assertion passed")

(let [runtime (create-ns 'demo.runtime-backend)]
  (intern runtime 'backend (fn [] 41))
  (load-string "(ns demo.runtime-backend)
                 (defn run [] (inc (backend)))
                 (defn run-qualified [] (inc (demo.runtime-backend/backend)))")
  (doseq [name '[run run-qualified]]
    (assert (= 42 ((ns-resolve runtime name)))))
  (intern runtime 'backend (fn [] "changed"))
  (doseq [name '[run run-qualified]]
    (assert (= :not-a-number
               (try ((ns-resolve runtime name))
                    (catch ClassCastException _ :not-a-number))))))
(println "Clojure/JVM same-namespace var bindings: 4 assertions passed")

(load-file "examples/callables.clj")
(load-file "examples/callables_bad.clj")
(assert (= [3 nil] (demo.callables/priorities)))
(assert (= [true false nil] (demo.callables/statuses)))
(assert (= "Ready" (demo.callables/selected-label false)))
(assert (= "Prêt" (demo.callables/selected-label true)))
(assert (nil? (demo.callables/explicit-nil)))
(assert (= "Fallback" (demo.callables/safe-label)))
(assert (= "ready" (apply :label [{:label "ready"}])))
(doseq [run [demo.callables-bad/missing-key demo.callables-bad/extra-default]]
  (assert (= :arity
             (try (run)
                  (catch clojure.lang.ArityException _ :arity)))))
(println "Clojure/JVM callable lookups: 9 assertions passed")

(assert (= "ready" (demo.callables/choose-stage)))
(assert (= "chosen" ([1 "chosen"] 4294967297)))
(assert (= "chosen" ([1 "chosen"] 01)))
(assert (= ["value"] (mapv (fn [entry] (entry 1)) {:a "value"})))
(assert (= :bounds (try (demo.callables-bad/outside-vector)
                         (catch IndexOutOfBoundsException _ :bounds))))
(assert (= :not-integer (try ([1] 0.0)
                              (catch IllegalArgumentException _ :not-integer))))
(println "Clojure/JVM vector invocation: 6 assertions passed")

(load-file "examples/reductions.clj")
(load-file "examples/reductions_bad.clj")
(assert (= 6 (demo.reductions/sum-values)))
(assert (= 6 (demo.reductions/total-in-map)))
(assert (= "last" (demo.reductions/pick-collected)))
(assert (= "a-b-c" (demo.reductions/append-labels)))
(assert (= 0 (demo.reductions/empty-count)))
(assert (= "single" (reduce (fn [x y] (+ x y)) ["single"])))
(assert (= "fallback" (reduce 7 "fallback" ())))
(assert (= :bad-reducer (try (reduce 7 0 [])
                             (catch ClassCastException _ :bad-reducer))))
(doseq [run [demo.reductions-bad/bad-element demo.reductions-bad/bad-transition]]
  (assert (= :not-a-number
             (try (run)
                  (catch ClassCastException _ :not-a-number)))))
(println "Clojure/JVM ordered reductions: 10 assertions passed")

(assert (= 0 (demo.reductions/sum-open [])))
(assert (= 6 (demo.reductions/sum-open [1 2 3])))
(assert (= ["default" "A" "B"] (demo.reductions/append-open ["A" "B"])))
(assert (= "last" (reduce (fn [_ item] (str item)) 0 (seq [1 "last"]))))
(println "Clojure/JVM open reductions: 4 assertions passed")

(assert (= 5 (demo.reductions/sum-before-label)))
(assert (reduced? (reduce + (reduced 1) [])))
(assert (reduced? (reduce + [(reduced 1)])))
(assert (reduced? (reduce (fn [] (reduced 1)) [])))
(assert (reduced? (reduce (fn [] (reduced 1)) nil)))
(assert (= "first" (unreduced (reduce (fn [_ x] (reduced (reduced x))) 0 ["first" 1]))))
(assert (= "only" (unreduced (ensure-reduced (reduced "only")))))
(println "Clojure/JVM early reduction results: 7 assertions passed")

(load-file "examples/reduce_kv.clj")
(assert (= 7 (demo.reduce-kv/total-usage {:first 2 :second 5})))
(assert (= 0 (demo.reduce-kv/total-usage {})))
(assert (= 5 (demo.reduce-kv/config-score {:label "abc" :count 2})))
(assert (= 12 (demo.reduce-kv/weighted-total [2 5])))
(assert (= [[0 "first"] [1 false]]
           (reduce-kv (fn [result index value] (conj result [index value]))
                      [] ["first" false])))
(assert (= "stop" (reduce-kv (fn [_ index value]
                               (if (zero? index) (reduced value) (inc value)))
                             nil ["stop" false])))
(assert (= :initial (reduce-kv 42 :initial nil)))
(println "Clojure/JVM key-value reduction: 7 assertions passed")

(assert (= "ADA" (demo.guards/normalize-name "Ada")))
(assert (= "UNKNOWN" (demo.guards/normalize-name nil)))
(assert (nil? (demo.guards/when-normalized nil)))
(assert (= "last" (when-not false 42 "last")))
(assert (nil? (if-not true "skipped")))
(assert (= :done (loop [again true] (if-not again :done (recur false)))))
(assert (= :done (loop [again true] (if again (when-not false (recur false)) :done))))
(println "Clojure/JVM negated conditionals: 7 assertions passed")

(assert (= 0 (demo.sequences/sum-with-empty-guard [])))
(assert (= 6 (demo.sequences/sum-with-empty-guard [1 2 3])))
(assert (empty? nil))
(assert (empty? (rest [1])))
(assert (false? (empty? [nil])))
(assert (false? (empty? {:x false})))
(assert (nil? (not-empty "")))
(assert (= [nil] (not-empty [nil])))
(assert (= ["Ada"] (filterv not-empty ["" "Ada"])))
(println "Clojure/JVM emptiness guards: 9 assertions passed")

(assert (= #{:received :queued} (set (demo.sequences/entry-keys))))
(assert (= "Ready" (demo.sequences/first-entry-label)))
(assert (= \x (demo.sequences/after-surrogates)))
(assert (nil? (next {:only 1})))
(assert (nil? (seq #{})))
(assert (= \A (first "Ada")))
(assert (= 2 (count (seq "🚀"))))
(assert (char? (nth (seq "🚀") 1)))
(println "Clojure/JVM maps, sets and string sequence views: 8 assertions passed")

(load-file "examples/keyword_aliases.clj")
(assert (= :demo.schema/status (demo.keyword-aliases/status-key)))
(assert (= :demo.keyword-aliases/ready (demo.keyword-aliases/local-key)))
(assert (= :invalid-token
           (try
             (load-file "examples/keyword_aliases_bad.clj")
             (catch clojure.lang.Compiler$CompilerException e
               (when (clojure.string/includes? (ex-message (ex-cause e))
                                               "Invalid token: ::missing/status")
                 :invalid-token)))))
(doseq [source ["#?(:clj :ok :cljs ::missing/status)" "::clojure.core/status"]]
  (assert (= :invalid-token
             (try
               (read-string {:read-cond :allow} source)
               (catch RuntimeException e
                 (when (clojure.string/includes? (ex-message e) "Invalid token")
                   :invalid-token))))))
(assert (= :invalid-edn
           (try
             (clojure.edn/read-string "::status")
             (catch RuntimeException _ :invalid-edn))))
(println "Clojure/JVM keyword aliases: 6 assertions passed")

(load-file "examples/quoted.clj")
(load-file "examples/quoted_bad.clj")
(assert (= "fallback" (demo.quoted/fallback-label)))
(assert (= 'demo.external/command (demo.quoted/command-tag)))
(assert (= [:online :offline] (demo.quoted/modes)))
(assert (= "guest" (demo.quoted/guest-label)))
(assert (= 'quote (first ''command)))
(assert (= :not-a-number
           (try
             (demo.quoted-bad/invalid-number)
             (catch ClassCastException _ :not-a-number))))
(println "Clojure/JVM quoted literal data: 6 assertions passed")

(load-file "examples/imports.clj")
(assert (= :request (demo.imports/request-stage)))
(assert (= java.io.FileNotFoundException (ns-resolve 'demo.imports 'FileNotFoundException)))
(assert (= java.lang.ArithmeticException (ns-resolve 'demo.imports 'ArithmeticException)))
(assert (= :collision
           (try
             (load-file "examples/imports_bad.clj")
             (catch clojure.lang.Compiler$CompilerException e
               (when (clojure.string/includes? (ex-message (ex-cause e)) "Date already refers")
                 :collision)))))
(println "Clojure/JVM class imports: 4 assertions passed")

(load-file "examples/lookup_order.clj")
(load-file "examples/lookup_order_bad.clj")
(assert (= :default (demo.lookup-order/eager-default)))
(assert (= :key (demo.lookup-order/first-path-error)))
(assert (= :missing (demo.lookup-order/scalar-fallback)))
(assert (true? (demo.lookup-order/string-index-present?)))
(assert (= :default (demo.lookup-order-bad/wrong-recovery)))
(assert (= :target
           (try (contains? (throw (ex-info "target" {:stage :target}))
                           (throw (ex-info "key" {:stage :key})))
                (catch ExceptionInfo e (:stage (ex-data e))))))
(assert (= "Missing required key: :missing"
           (try (req! 42 :missing)
                (catch IllegalArgumentException e (ex-message e)))))
(println "Clojure/JVM lookup evaluation order: 7 assertions passed")

(assert (= :value (demo.lookup-order/rejected-write)))
(assert (= :ready (demo.lookup-order/saved-status)))
(assert (= :value
           (try (assoc 42 :status (throw (ex-info "value" {:stage :value})))
                (catch ExceptionInfo e (:stage (ex-data e))))))
(assert (= :wrong-parent
           (try (assoc-in {:request 7} [:request :status] :ready)
                (catch ClassCastException _ :wrong-parent))))
(println "Clojure/JVM map write evaluation order: 4 assertions passed")

(load-file "examples/helper_effects.clj")
(assert (= :missing (demo.helper-effects/request-stage)))
(assert (= :ready (demo.helper-effects/request-status)))
(assert (= :param (demo.helper-effects/rejected-argument)))
(load-file "examples/helper_effects_bad.clj")
(assert (= :missing (demo.helper-effects-bad/request-stage)))
(println "Clojure/JVM private helper effects: 4 assertions passed")

(load-file "examples/list_accumulators.clj")
(load-file "examples/list_accumulators_bad.clj")
(assert (= '("Ada" "Eve") (demo.list-accumulators/reverse-names ["Eve" "Ada"])))
(assert (seq? (demo.list-accumulators/reverse-names [])))
(assert (= :paused (demo.list-accumulators/second-status)))
(assert (= :requested (demo.list-accumulators/head-status)))
(assert (= '(nil :next) (cons nil '(:next))))
(assert (= :invalid-tail
           (try (demo.list-accumulators-bad/invalid-tail)
                (catch IllegalArgumentException _ :invalid-tail))))
(assert (= '("Eve" "Ada") (demo.list-accumulators/copy-names ["Eve" "Ada"])))
(assert (empty? (demo.list-accumulators/copy-names [])))
(println "Clojure/JVM list accumulators: 8 assertions passed")

(assert (= 7 (demo.reductions/sum-line-items)))
(assert (= 3 (demo.reductions/total-utf16-units)))
(assert (= 10 (demo.reductions/sum-distinct)))
(assert (= 7 (demo.reductions/singleton-price)))
(assert (= :seed (reduce 42 :seed {})))
(assert (= :seed (reduce 42 :seed #{})))
(assert (= \a (reduce 42 "a")))
(assert (= :wrong-value
           (try (demo.reductions-bad/bad-line-item)
                (catch ClassCastException _ :wrong-value))))
(println "Clojure/JVM direct collection reductions: 8 assertions passed")

(assert (= :row (demo.vectors/rejected-row)))
(assert (= [] (demo.vectors/filtered-empty)))
(assert (= :row
           (try (filterv (fn [_] (throw (ex-info "Invalid row" {:stage :row}))) [:only])
                (catch ExceptionInfo e (:stage (ex-data e))))))
(assert (= []
           (mapv (fn [_ _] (throw (ex-info "Unused" {}))) [1 2] [])))
(println "Clojure/JVM vector callback exits: 4 assertions passed")

(assert (= 140 (demo.ordered-paths/computed-quote {"SKU-17" {:retail 140}} "SKU-17")))
(assert (= {"SKU-17" {:retail 141}}
           (demo.ordered-paths/adjust-stored-path {} "SKU-17" 140)))
(assert (= :target (get-in :target nil :fallback)))
(assert (= :target (get-in :target (list) :fallback)))
(assert (nil? (get-in {:status nil} (list :status) :missing)))
(println "Clojure/JVM sequential paths: 5 assertions passed")

(assert (= :ran (demo.guards/first-flag)))
(assert (= :pending (demo.guards/next-status)))
(assert (= [nil :ran] (when-first [item [nil :next]] [item :ran])))
(assert (nil? (when-first [item []] item)))
(assert (= 5 (when-first [[_ value] {:cost 5}] value)))
(assert (= :invalid
           (try (when-first [item 42] item)
                (catch IllegalArgumentException _ :invalid))))
(println "Clojure/JVM when-first: 6 assertions passed")

(load-file "examples/numbers.clj")
(assert (== 29 (demo.numbers/invoice-total)))
(assert (== 131 (demo.numbers/sample-id)))
(assert (= '(1 3 (quote a) 4 %b) (read-string "(1#_2 3'a 4%b)")))
(assert (number? (read-string "0xFFFFFFFFFFFFFFFFFFFF")))
(assert (number? (read-string "08M")))
(doseq [token ["08" "2r10N" "1/00" "1e10000000000M"]]
  (assert (try (read-string token) false (catch RuntimeException _ true))))
(println "Clojure/JVM numeric literals: 9 assertions passed")

(assert (= :pending (demo.guards/next-status)))
(assert (= :ready :ready :ready))
(assert (not (= :ready :pending)))
(assert (not (= nil false)))
(assert (not (= \a "a")))
(assert (= 1 1N))
(assert (not (= 1 1.0)))
(println "Clojure/JVM scalar equality: 7 assertions passed")

(load-file "examples/generic_callbacks.clj")
(assert (= 3 (demo.generic-callbacks/label-length " foo ")))
(assert (= [:found "p-42"] (demo.generic-callbacks/patient-response "p-42")))
(assert (= {:names ["Ada" "Lin"]}
           (demo.generic-callbacks/list-response [{:name "Ada"} {:name "Lin"}])))
(assert (= ["2" "3"] (demo.generic-callbacks/format-values [1 2])))
(println "Clojure/JVM generic callbacks: 4 assertions passed")

(let [{:keys [result-code message-fn]} {:result-code 0 :message-fn (fn [] "formatted")}]
  (assert (= [0 "formatted"] [result-code (message-fn)])))
(println "Clojure/JVM result callback: 1 assertion passed")

(let [choose (fn [_ fallback] (nth fallback 0))
      run-on-fallback (fn [_ fallback f] (f (nth fallback 0)))]
  (assert (= "known" (choose [] ["known"])))
  (assert (= "ext" (run-on-fallback [] ["text"] #(subs % 1)))))
(println "Clojure/JVM fallback vectors and contextual calls: 2 assertions passed")

(load-file "examples/map_boundary/runtime/vendor/patient_store.clj")
(load-file "examples/map_boundary/src/map_boundary/model.clj")
(load-file "examples/map_boundary/src/map_boundary/response.clj")
(load-file "examples/map_boundary/src/map_boundary/http.clj")
(assert (= {:status 200 :body "Ada"} (map-boundary.http/show-patient {:id "p-1"})))
(assert (= {:status 200 :body "Unknown"} (map-boundary.http/show-patient {:id "p-2"})))
(assert (= {:status 200 :body nil} (map-boundary.http/show-patient {:id "p-3"})))
(assert (= {:status 404 :body nil} (map-boundary.http/show-patient {:id "p-404"})))
(println "Clojure/JVM map boundary: 4 assertions passed")

(load-file "examples/generic_zip.clj")
(assert (= [{:label "first" :value 7}] (demo.generic-zip/readings ["first" "ignored"] [7])))
(assert (= ["Visits: 8"] (demo.generic-zip/describe [" Visits "] [7 9])))
(assert (= [] (demo.generic-zip/readings ["extra"] [])))
(assert (= "Today" (demo.generic-zip/label-or-empty {:daily " Today "})))
(assert (= "" (demo.generic-zip/label-or-empty {:weekly "Later"})))
(println "Clojure/JVM generic zip and nullable results: 5 assertions passed")

(load-file "examples/showcase.clj")
(let [batch (demo.showcase/sample-batch)]
  (assert (= {:status 200 :body {:title "SHIFT A" :score 14
                                 :rows [{:text "Meter reading" :score 14}
                                        {:text "CHECK" :score 0}]}}
             (demo.showcase/summarize-batch batch)))
  (assert (= {:status 422 :body {:title "Tenant is required" :score 0 :rows []}}
             (demo.showcase/summarize-batch (update batch :labels dissoc :tenant))))
  (assert (= "North hub" (get-in (demo.showcase/summarize-batch (assoc batch :custom-title? false)) [:body :title])))
  (assert (= "North hub" (get-in (demo.showcase/summarize-batch (assoc-in batch [:labels :title] nil)) [:body :title])))
  (assert (= 1 (count (get-in (demo.showcase/summarize-batch (assoc batch :weights [2])) [:body :rows])))))
(println "Clojure/JVM showcase: 5 assertions passed")

(load-file "examples/regex_split.cljc")
(assert (= ["a" "b"] (demo.query-string/fields "a=b" #"=")))
(assert (nil? (demo.query-string/field-value "a")))
(assert (= "" (demo.query-string/field-value "a=")))
(assert (= "b=c" (demo.query-string/field-value "a=b=c")))
(println "Clojure/JVM regex split: 4 assertions passed")

(assert (= "a/" (re-find #"\A.*/" "a/b")))
(assert (nil? (re-find #"\A.*/" "ab")))
(assert (= ["a" "a" nil] (re-matches #"(a)(b)?" "a")))
(assert (= "a" (re-find (re-matcher #"a" "ab"))))
(let [jar-pattern #"^(.*\.jar):(.*)"]
  (assert (= ["file.jar:entry.clj" "file.jar" "entry.clj"]
             (re-find jar-pattern "file.jar:entry.clj")))
  (assert (nil? (re-find jar-pattern "missing"))))
(assert (= [0 1] (mapv #(.groupCount (.matcher % "")) [#"\c(" #"\c\(a)"])))
(println "Clojure/JVM regex matching: 7 assertions passed")

(let [boxed-false (Boolean. false)]
  (assert (= :then (if boxed-false :then :else)))
  (assert (false? (boolean boxed-false)))
  (assert (false? (apply boolean [boxed-false])))
  (assert (= [false false true true true true true]
             (mapv boolean [nil false true 0 "" [] #{}]))))
(println "Clojure/JVM boolean conversion: 4 assertions passed")

(assert (= "a" (subs "😀a" 2)))
(assert (= "😀" (subs "😀a" 0 2)))
(assert (Character/isHighSurrogate (.charAt (subs "😀a" 0 1) 0)))
(assert (try (subs "😀" 3) false (catch StringIndexOutOfBoundsException _ true)))
(println "Clojure/JVM substring bounds: 4 assertions passed")

(assert (try (do (subs nil 0) (clojure.string/upper-case))
             false
             (catch NullPointerException _ true)))
(assert (try (clojure.string/upper-case (subs nil 0) "extra")
             false
             (catch NullPointerException _ true)))
(assert (try (clojure.string/upper-case)
             false
             (catch clojure.lang.ArityException _ true)))
(assert (try (eval '(fn [] (do (subs nil 0) (inc))))
             false
             (catch clojure.lang.Compiler$CompilerException e
               (instance? clojure.lang.ArityException (ex-cause e)))))
(assert (try (do (throw nil) (name))
             false
             (catch NullPointerException _ true)))
(assert (try (do (throw nil) (apply count []))
             false
             (catch NullPointerException _ true)))
(assert (try (apply count [])
             false
             (catch clojure.lang.ArityException _ true)))
(defn- arity-smoke-helper [x] x)
(assert (try (do (throw nil) (arity-smoke-helper))
             false
             (catch NullPointerException _ true)))
(assert (try (arity-smoke-helper (throw nil) :extra)
             false
             (catch NullPointerException _ true)))
(assert (= :arity (try (arity-smoke-helper)
                      (catch NullPointerException _ :wrong)
                      (catch clojure.lang.ArityException _ :arity))))
(println "Clojure/JVM call arity phases: 10 assertions passed")

(doseq [call '[(get) (req!) (get-in) (contains?)]]
  (let [reached (eval (list 'fn [] call))
        skipped (eval (list 'fn [] (list 'do '(throw nil) call)))]
    (assert (try (reached) false (catch clojure.lang.ArityException _ true)))
    (assert (try (skipped) false (catch NullPointerException _ true)))))
(println "Clojure/JVM map lookup arity: 8 assertions passed")

(let [run (eval '(fn []
                   (do (throw nil)
                       {:named (fn again [x] (again x))
                        :shorthand #(reduce)
                        :raw (fn* [x] x)})))]
  (assert (try (run) false (catch NullPointerException _ true))))
(let [stored (eval '(fn [] {:lookup (fn [] (get)) :increment #(inc "text")}))]
  (assert (every? fn? (vals (stored)))))
(doseq [literal '[(fn [x] (no-such-closure-target x))
                  #(no-such-closure-target %)
                  (fn* [x] (no-such-closure-target x))]
        body [{:run literal} (list 'do '(throw nil) literal)]]
  (assert (try (eval (list 'fn [] body))
               false
               (catch clojure.lang.Compiler$CompilerException e
                 (= "Unable to resolve symbol: no-such-closure-target in this context"
                    (ex-message (ex-cause e)))))))
(println "Clojure/JVM compiler checks in stored closures: 8 assertions passed")

(let [file (new java.io.File "document.txt")]
  (assert (= "document.txt" (.getName file) (. file getName) (.. file getName))))
(assert (= "document.txt"
           (eval '(. (java.io.File. "document.txt") (getName) ignored-member-tail))))
(assert (= 4 (eval '(. "text" (length) ignored-member-tail))))
(assert (try (eval '(fn [] (.getName)))
             false
             (catch clojure.lang.Compiler$CompilerException e
               (clojure.string/includes? (ex-message (ex-cause e)) "Malformed member expression"))))
(defmacro .opaque-smoke-member [_] :macro)
(assert (= :macro (.opaque-smoke-member ignored-member-argument)))
(println "Clojure/JVM interop syntax: 5 assertions passed")

(create-ns 'parenox.smoke.interop-provider)
(alias 'interop-provider 'parenox.smoke.interop-provider)
(doseq [[body name] '[[(. (interop-provider/receiver "abc") (length))
                        interop-provider/receiver]
                       [(. "abc" (charAt (interop-provider/nested-index))
                                 (interop-provider/ignored-tail))
                        interop-provider/nested-index]
                       [(. "abc" charAt (interop-provider/flat-index))
                        interop-provider/flat-index]
                       [(new java.io.File (interop-provider/path))
                        interop-provider/path]]]
  (assert (try (eval (list 'fn [] body))
               false
               (catch clojure.lang.Compiler$CompilerException e
                 (= (str "No such var: " name) (ex-message (ex-cause e)))))))
(assert (= \b (eval '(. "abc" (charAt 1) (interop-provider/ignored-tail)))))
(println "Clojure/JVM project references in interop operands: 5 assertions passed")

(create-ns 'parenox.smoke.external-scope)
(binding [*ns* (the-ns 'parenox.smoke.external-scope)]
  (clojure.core/refer 'clojure.core)
  (eval '(defmacro with-value [[name value] body _] `(let [~name ~value] ~body))))
(alias 'external-scope 'parenox.smoke.external-scope)
(assert (= "bc" (eval '(external-scope/with-value [local "abc"]
                          (subs local 1)
                          (subs false 0)))))
(binding [*ns* (the-ns 'parenox.smoke.external-scope)]
  (eval '(defn with-value [binding body extra] body)))
(assert (try (eval '(external-scope/with-value [local "abc"] (subs local 1) (subs false 0)))
             false
             (catch clojure.lang.Compiler$CompilerException e
               (= "Unable to resolve symbol: local in this context" (ex-message (ex-cause e))))))
(println "Clojure/JVM unknown external operand scope: 2 assertions passed")

(let [provider 'parenox.smoke.wildcard-provider
      consumer 'parenox.smoke.wildcard-consumer]
  (binding [*ns* (create-ns provider)]
    (clojure.core/refer 'clojure.core :exclude '[subs /])
    (eval '(defmacro with-value [[name value] body _] `(let [~name ~value] ~body)))
    (eval '(defmacro .length [_] :from-referral))
    (eval '(defn subs [s n] :from-referral))
    (eval '(defn / [a b] :from-referral)))
  (dosync (alter @#'clojure.core/*loaded-libs* conj provider))
  (binding [*ns* *ns* *err* (java.io.StringWriter.)]
    (eval (list 'ns consumer (list :require [provider :refer :all]))))
  (binding [*ns* (the-ns consumer)]
    (assert (= "bc" (eval '(with-value [local "abc"]
                            (clojure.core/subs local 1)
                            (clojure.core/subs false 0)))))
    (assert (= "bc" (eval '(clojure.core/subs "abc" 1))))
    (doseq [form '[(subs "abc" 0) (/ 6 2)
                   (clojure.core/apply subs ["abc" 0]) (.length "abc")]]
      (assert (= :from-referral (eval form))))))
(println "Clojure/JVM wildcard referral scope: 6 assertions passed")

(import '[java.io File])
(assert (= java.io.File File))
(assert (= java.lang.String String))
(assert (= 1 (let [File 1] File)))
(println "Clojure/JVM class values: 3 assertions passed")

(assert (= "42" (String/valueOf 42)))
(assert (= 4 (String/.length "text")))
(assert (= "document.txt" (.getName (File/new "document.txt"))))
(assert (= 2147483647 Integer/MAX_VALUE))
(create-ns 'String)
(try
  (intern 'String 'valueOf (constantly :namespace))
  (assert (= :namespace (eval '(String/valueOf 42))))
  (assert (try (eval '(String/.length "text"))
               false
               (catch clojure.lang.Compiler$CompilerException e
                 (= "No such var: String/.length" (ex-message (ex-cause e))))))
  (finally (remove-ns 'String)))
(println "Clojure/JVM class-qualified members: 6 assertions passed")

(assert (= \a (. "😀a" (charAt 2))))
(assert (Character/isHighSurrogate (.charAt "😀" 0)))
(assert (try (.charAt "😀" 2) false (catch StringIndexOutOfBoundsException _ true)))
(println "Clojure/JVM charAt bounds: 3 assertions passed")

(assert (= "" (clojure.string/join nil)))
(assert (= ",b" (clojure.string/join \, [nil "b"])))
(assert (= "a:sep:b" (clojure.string/join :sep ["a" ":b"])))
(assert (try (clojure.string/join \/ 42) false (catch IllegalArgumentException _ true)))
(println "Clojure/JVM join inputs: 4 assertions passed")

(doseq [read [read-string clojure.edn/read-string]]
  (assert (= 4 (count (read "#{1 1.0 1M -1}"))))
  (assert (try (read "#{0xffN 16rFF}") false (catch RuntimeException _ true)))
  (assert (try (read "#{59 36r1N}") false (catch RuntimeException _ true)))
  (assert (try (read "{-10 :first -0xAN :second}") false (catch RuntimeException _ true))))
(println "Clojure/JVM and EDN integer equality: 8 assertions passed")

(assert (= "" (demo.query-string/normalized-value {})))
(assert (= "" (demo.query-string/normalized-value {:value nil})))
(assert (= "" (demo.query-string/normalized-value {:value " \t"})))
(assert (= "a b" (demo.query-string/normalized-value {:value "a+b"})))
(assert (= "a+b" (clojure.string/replace "a-b" \- \+)))
(println "Clojure/JVM blank strings and replacements: 5 assertions passed")

(load-file "examples/json_fields.cljc")
(assert (= "Anonymous" (demo.json-fields/display-name {})))
(assert (= "Unknown" (demo.json-fields/display-name {"name" nil})))
(assert (= "Ada" (demo.json-fields/required-name {"name" "Ada" "schema" nil})))
(assert (= "Missing required key: \"schema\""
           (try (demo.json-fields/required-name {"name" "Ada"})
                (catch Exception e (.getMessage e)))))
(println "Clojure/JVM destructuring string keys: 4 assertions passed")

(load-file "examples/jvm_strings.cljc")
(assert (= "pair" (demo.jvm-strings/supplementary-length)))
(assert (= 0xD83D (demo.jvm-strings/maybe-code-unit {"name" "🙂"} 0)))
(assert (nil? (demo.jvm-strings/maybe-code-unit {"name" nil} 0)))
(println "Clojure/JVM String member calls: 3 assertions passed")

(doseq [read [read-string edn/read-string]]
  (assert (= (assoc {:stock/sku "id" :category :input :vendor/price 1} 'stock/code 7 'raw 9)
             (read "#:stock {:sku \"id\" :_/category :input :vendor/price 1 code 7 _/raw 9}")))
  (assert (try (read "#:stock{:sku 1 :stock/sku 2}") false (catch RuntimeException _ true))))
(println "Clojure/JVM and EDN namespaced maps: 4 assertions passed")

(load-file "examples/namespaced_keys.cljc")
(assert (= "A-1" (demo.namespaced-keys/sku)))
(assert (= :public (demo.namespaced-keys/access-level)))
(println "Clojure/JVM auto-resolved namespaced maps: 2 assertions passed")

(doseq [read [read-string edn/read-string]]
  (assert (= [27 91 51 56 59 53 59] (mapv int (read "\"\\033[38;5;\""))))
  (assert (= [255] (mapv int (read "\"\\377\""))))
  (assert (try (read "\"\\400\"") false (catch RuntimeException _ true))))
(println "Clojure/JVM and EDN octal string escapes: 6 assertions passed")
