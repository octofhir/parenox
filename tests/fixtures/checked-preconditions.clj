(ns checked-preconditions)

;; JVM reference: clojure -M tests/fixtures/checked-preconditions.clj

(defn assert-failure [f expected]
  (let [error (try (f) nil (catch AssertionError e e))]
    (assert (= AssertionError (class error)))
    (assert (= expected (ex-message error)))))

(defn destructured [[text] {:keys [count]}]
  {:pre [(string? text) (pos? count)]}
  [text count])

(assert (= ["a" 2] (destructured ["a"] {:count 2})))
(assert-failure #(destructured [nil] {:count 0}) "Assert failed: (string? text)")
(assert-failure #(destructured ["a"] {:count 0}) "Assert failed: (pos? count)")

(defn from-metadata ^{:pre [(string? text)]} [text]
  {:pre [false] :text text})

(defn explicit-wins ^{:pre [false]} [value]
  {:pre [(number? value)]}
  value)

(defn empty-map-wins ^{:pre [false]} []
  {}
  :body)

(defn unrelated-map-wins ^{:pre [false]} []
  {:example true}
  :body)

(assert (= {:pre [false] :text "a"} (from-metadata "a")))
(assert-failure #(from-metadata nil) "Assert failed: (string? text)")
(assert (= 42 (explicit-wins 42)))
(assert-failure #(explicit-wins "a") "Assert failed: (number? value)")
(assert (= :body (empty-map-wins)))
(assert (= :body (unrelated-map-wins)))
(assert (= {:pre [false]} ((fn [] {:pre [false]}))))

(assert (= :body ((fn [] {:pre nil} :body))))
(assert (= :body ((fn [] {:pre false} :body))))
(assert (= :body ((fn [] {:pre []} :body))))
(assert (= :body ((fn [] {:pre [0 [] {} '() ""]} :body))))
(assert-failure (fn [] {:pre [nil]} :body) "Assert failed: nil")
(assert-failure (fn [] {:pre [false]} :body) "Assert failed: false")

(defn through-recur [[n] calls]
  {:pre [(do (swap! calls conj [:pre n]) (pos? n))]}
  (swap! calls conj [:body n])
  (recur [(dec n)] calls))

(let [calls (atom [])]
  (assert-failure #(through-recur [2] calls)
                  "Assert failed: (do (swap! calls conj [:pre n]) (pos? n))")
  (assert (= [[:pre 2] [:body 2] [:pre 1] [:body 1] [:pre 0]] @calls)))

;; The compile-time value of *assert* selects whether a check is emitted.
(let [form '(fn [calls] {:pre [(do (swap! calls conj :pre) false)]} :body)
      enabled (binding [*assert* true] (eval form))
      disabled (binding [*assert* false] (eval form))
      calls (atom [])]
  (binding [*assert* false]
    (assert-failure #(enabled calls) "Assert failed: (do (swap! calls conj :pre) false)"))
  (assert (= [:pre] @calls))
  (binding [*assert* true]
    (assert (= :body (disabled calls))))
  (assert (= [:pre] @calls)))

(prn {:clojure (clojure-version) :checked-preconditions :ok})
