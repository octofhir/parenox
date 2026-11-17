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
