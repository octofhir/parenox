(ns spec-range
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-range.clj
(doseq [register [#(s/def ::start ::position) #(s/def ::end ::position)]]
  (let [before (s/registry)
        error (try (register) nil (catch Exception error error))]
    (assert (= Exception (class error)))
    (assert (= (str "Unable to resolve spec: " ::position) (ex-message error)))
    (assert (identical? before (s/registry)))))
(assert (nil? (s/get-spec ::start)))
(assert (nil? (s/get-spec ::end)))

;; Exact declarations from lsp4clj 2.0.1, coercer.clj lines 31–36.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))

;; Range assertions.
(defn outcome [f spec input]
  (try (f spec input) (catch Exception error error)))

(defn assert-invalid [input]
  (assert (= ::s/invalid (s/conform ::range input)))
  (assert (false? (s/valid? ::range input))))

(defn assert-throws [input expected]
  (doseq [f [s/conform s/valid?]]
    (assert (= expected (class (outcome f ::range input))))))

(defn assert-position [position line character]
  (assert (= line (:line position)))
  (assert (= character (:character position)))
  (assert (instance? Integer (:line position)))
  (assert (instance? Integer (:character position))))

(def start {:line -2147483648N :character 0})
(def end {:line 2147483647 :character (biginteger "1")})
(def input {:start start :end end})

(assert (= ::position (s/get-spec ::start) (s/get-spec ::end)))
(let [extra (Object.)
      result (s/conform ::range (assoc input :extra extra ::start end ::end start))]
  (assert-position (:start result) -2147483648 0)
  (assert-position (:end result) 2147483647 1)
  (assert-position (::start result) 2147483647 1)
  (assert-position (::end result) -2147483648 0)
  (assert (identical? extra (:extra result))))
(assert (true? (s/valid? ::range input)))

(defrecord SourcePosition [line character])
(defrecord SourceRange [start end])
(let [record (->SourceRange (map->SourcePosition start) (map->SourcePosition end))
      result (s/conform ::range record)]
  (assert (instance? SourceRange result))
  (assert (instance? SourcePosition (:start result)))
  (assert (instance? SourcePosition (:end result)))
  (assert-position (:start result) -2147483648 0)
  (assert-position (:end result) 2147483647 1)
  (assert (true? (s/valid? ::range record))))

(doseq [value [nil false ::s/invalid [] (java.util.HashMap. input)]]
  (assert-invalid value))
(doseq [endpoint [:start :end]
        value [nil false ::s/invalid {} {:line 0} {:character 1}
               {:line nil :character 1} {:line 0 :character false}
               {:line ::s/invalid :character 1}]]
  (assert-invalid (assoc input endpoint value)))
(assert-invalid {::start start ::end end})

(doseq [endpoint [:start :end]
        [value expected] [[2147483648 ArithmeticException]
                          [-2147483649N ArithmeticException]
                          [9223372036854775808N IllegalArgumentException]]]
  (let [changed (assoc-in input [endpoint :line] value)]
    (assert-throws changed expected)
    (assert-invalid (dissoc changed (if (= endpoint :start) :end :start)))
    (assert-invalid (update changed endpoint dissoc :character))))

(let [overflow {:line 2147483648 :character 0}]
  (assert-invalid (array-map :start false :end overflow))
  (assert-throws (array-map :end overflow :start false) ArithmeticException)
  (assert-invalid (assoc input :start (array-map :character nil :line 2147483648)))
  (assert-throws (assoc input :start (array-map :line 2147483648 :character nil))
                 ArithmeticException))

(let [value (proxy [java.math.BigInteger] ["2147483648"]
              (bitLength [] 1)
              (longValue [] 0))
      input (assoc-in input [:start :line] value)
      result (s/conform ::range input)]
  (assert (= "2147483648" (str value)))
  (assert (true? (s/valid? ::range input)))
  (assert (identical? value (get-in input [:start :line])))
  (assert-position (:start result) 0 0))

(s/def ::position (s/conformer (fn [value] {:wrapped value})))
(doseq [alias [::start ::end]]
  (assert (= ::position (s/get-spec alias)))
  (assert (= {:wrapped false} (s/conform alias false)))
  (assert (true? (s/valid? alias false))))
(let [changed {:start false :end ::s/invalid}]
  (assert (= {:start {:wrapped false} :end {:wrapped ::s/invalid}}
             (s/conform ::range changed)))
  (assert (true? (s/valid? ::range changed))))

(s/def ::position nil)
(doseq [f [s/conform s/valid?]]
  (doseq [alias [::start ::end]]
    (assert (= ::position (s/get-spec alias)))
    (assert (= (str "Unable to resolve spec: " alias)
               (ex-message (outcome f alias start)))))
  (assert (= (str "Unable to resolve spec: " ::position)
             (ex-message (outcome f ::range input)))))
(assert-invalid {:start start})

(s/def ::position (s/keys :req-un [::line ::character]))
(assert-position (:start (s/conform ::range input)) -2147483648 0)
(s/def ::start nil)
(let [changed {:start false :end end}
      result (s/conform ::range changed)]
  (assert (false? (:start result)))
  (assert-position (:end result) 2147483647 1)
  (assert (true? (s/valid? ::range changed))))
(s/def ::start ::position)
(assert-invalid {:start false :end end})

(prn {:clojure (clojure-version) :spec-range :ok})
