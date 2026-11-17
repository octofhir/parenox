;; Optional independent smoke check against actual Clojure/JVM.
;; Run from the project root: clojure -M scripts/runtime_smoke.clj
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
(println "Clojure/JVM sets: 7 assertions passed")

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
