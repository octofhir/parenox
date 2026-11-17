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
