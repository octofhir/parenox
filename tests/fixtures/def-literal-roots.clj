(ns def-literal-roots)

;; JVM reference: clojure -M tests/fixtures/def-literal-roots.clj

(def ^:private private-root ^:origin [:private])
(def public-root :initial)
(def nil-root nil)
(def false-root false)
(def captured {:map {:private private-root :public public-root}
               :vector [private-root public-root nil-root false-root]
               :set #{private-root public-root nil-root false-root}})

(assert (= {:map {:private [:private] :public :initial}
            :vector [[:private] :initial nil false]
            :set #{[:private] :initial nil false}}
           captured))
(assert (:private (meta #'private-root)))
(assert (:origin (meta (get-in captured [:map :private]))))
(assert (identical? private-root (get-in captured [:vector 0])))

(def public-root :changed)
(assert (= :initial (get-in captured [:map :public])))
(assert (= :changed public-root))

;; A later mutation affects subsequent evaluation while the earlier capture survives.
(alter-var-root #'public-root (constantly :mutated))
(def after-mutation [public-root])
(assert (= [:mutated] after-mutation))
(assert (= :initial (get-in captured [:vector 1])))

(defn eval-error [form]
  (try (eval form)
       nil
       (catch clojure.lang.Compiler$CompilerException e (ex-cause e))))

;; A future def and a previously declared but unbound Var behave differently.
(let [error (eval-error '(def bad-forward [later-root]))]
  (assert (instance? RuntimeException error))
  (assert (= "Unable to resolve symbol: later-root in this context" (ex-message error))))
(def later-root :defined)

(declare pending-root)
(def captured-unbound [pending-root])
(assert (not (bound? #'pending-root)))
(assert (instance? clojure.lang.Var$Unbound (first captured-unbound)))
(assert (identical? (var-get #'pending-root) (first captured-unbound)))
(def pending-root :bound)
(assert (instance? clojure.lang.Var$Unbound (first captured-unbound)))

;; Equal evaluated keys and members fail even when the source names differ.
(def small-root 1)
(def big-root 1N)
(doseq [form ['(def equal-map-keys {small-root :first big-root :second})
              '(def equal-set-members #{small-root big-root})]]
  (let [error (eval-error form)]
    (assert (instance? IllegalArgumentException error))
    (assert (= "Duplicate key: 1" (ex-message error)))))

;; Runtime bindings are outside the adjacent literal-root model.
(def ^:dynamic *dynamic-root* :root)
(binding [*dynamic-root* :bound]
  (def captured-binding [*dynamic-root*]))
(assert (= [:bound] captured-binding))
(assert (= :root *dynamic-root*))

(prn {:clojure (clojure-version) :def-literal-roots :ok})
