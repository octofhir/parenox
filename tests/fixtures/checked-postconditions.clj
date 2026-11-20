(ns checked-postconditions)

;; JVM reference: clojure -M tests/fixtures/checked-postconditions.clj

(defn assert-failure [f expected]
  (let [error (try (f) nil (catch AssertionError e e))]
    (assert (= AssertionError (class error)))
    (assert (= expected (ex-message error)))))

(defn percent-binding [%]
  {:pre [(= % :argument)]
   :post [(= % :result)]}
  (if (= % :argument) :result :wrong))

(assert (= :result (percent-binding :argument)))

(defn ordered [calls value]
  {:post [(do (swap! calls conj [:first %]) (vector? %))
          (do (swap! calls conj [:second %]) (seq %))]}
  (swap! calls conj :body-start)
  (swap! calls conj :body-end)
  value)

(let [calls (atom [])
      value (with-meta [:result] {:from :body})]
  (assert (identical? value (ordered calls value)))
  (assert (= [:body-start :body-end [:first value] [:second value]] @calls))
  (reset! calls [])
  (assert-failure #(ordered calls 42) "Assert failed: (do (swap! calls conj [:first %]) (vector? %))")
  (assert (= [:body-start :body-end [:first 42]] @calls)))

(defn gated [calls allowed? failure]
  {:pre [(do (swap! calls conj :pre) allowed?)]
   :post [(do (swap! calls conj :post) true)]}
  (swap! calls conj :body)
  (throw failure))

(let [calls (atom [])
      failure (ex-info "body failed" {})]
  (assert-failure #(gated calls false failure) "Assert failed: (do (swap! calls conj :pre) allowed?)")
  (assert (= [:pre] @calls))
  (reset! calls [])
  (assert (identical? failure (try (gated calls true failure) (catch Exception e e))))
  (assert (= [:pre :body] @calls)))

(defn metadata-post ^{:post [(keyword? %)]} [value] value)
(defn explicit-wins ^{:post [false]} [] {:post [(= % :body)]} :body)
(defn empty-map-wins ^{:post [false]} [] {} :body)
(defn lone-map ^{:post [(map? %)]} [] {:post [false]})

(assert (= :body (metadata-post :body)))
(assert-failure #(metadata-post 42) "Assert failed: (keyword? %)")
(assert (= :body (explicit-wins)))
(assert (= :body (empty-map-wins)))
(assert (= {:post [false]} (lone-map)))

(let [form '(fn [calls] {:post [(do (swap! calls conj :post) false)]}
              (swap! calls conj :body)
              :value)
      enabled (binding [*assert* true] (eval form))
      disabled (binding [*assert* false] (eval form))
      calls (atom [])]
  (binding [*assert* false]
    (assert-failure #(enabled calls) "Assert failed: (do (swap! calls conj :post) false)"))
  (assert (= [:body :post] @calls))
  (reset! calls [])
  (binding [*assert* true]
    (assert (= :value (disabled calls))))
  (assert (= [:body] @calls)))

;; Even an empty post vector introduces a binding for the body result.
(doseq [mode [true false]]
  (binding [*assert* mode]
    (let [error (try (eval '(fn [n] {:post []} (if (zero? n) n (recur (dec n)))))
                     nil
                     (catch clojure.lang.Compiler$CompilerException e (ex-cause e)))
          nested (eval '(fn [n] {:post []} (loop [i n] (if (zero? i) i (recur (dec i))))))]
      (assert (instance? UnsupportedOperationException error))
      (assert (= "Can only recur from tail position" (ex-message error)))
      (assert (= 0 (nested 3))))))

(prn {:clojure (clojure-version) :checked-postconditions :ok})
