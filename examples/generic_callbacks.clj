(ns demo.generic-callbacks
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn transform
  {:t/sig '(t/all [a] [(s/fspec :args (s/cat :value a) :ret a) a :-> a])}
  [f value]
  (f value))

(defn next-id {:t/sig '[number? :-> number?]} [id]
  (transform #(inc %) id))

(defn upper-label {:t/sig '[string? :-> string?]} [label]
  (transform (fn [value] (str/upper-case value)) label))

(defn forward {:t/sig '(t/all [a] [a :-> a])} [value]
  (transform (fn [input] input) value))

(defn convert
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :value a) :ret b) a :-> b])}
  [f value]
  (f value))

(defn label-length {:t/sig '[string? :-> number?]} [label]
  (convert #(count (str/trim %)) label))

(defn patient-response {:t/sig '[string? :-> (s/tuple #{:found} string?)]} [id]
  (convert (fn [patient-id] [:found patient-id]) id))

(defn map-values
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :element a) :ret b)
                        (s/coll-of a :kind vector?) :-> (s/coll-of b :kind vector?)])}
  [f values]
  (mapv f values))

(s/def ::name string?)
(s/def ::patient (s/keys :req-un [::name]))
(s/def ::patients (s/coll-of ::patient :kind vector?))
(s/def ::names (s/coll-of string? :kind vector?))
(s/def ::response (s/keys :req-un [::names]))

(defn list-response {:t/sig '[::patients :-> ::response]} [patients]
  {:names (map-values #(:name %) patients)})

(defn number-label
  {:t/sig '(t/all [(a :< number?)] [a :-> string?])}
  [value]
  (str (inc value)))

(defn numeric-labels
  {:t/sig '(t/all [(a :< number?) (b :< string?)]
             [(s/fspec :args (s/cat :value a) :ret b)
              (s/coll-of a :kind vector?) :-> (s/coll-of b :kind vector?)])}
  [render values]
  (map-values render values))

(defn format-values {:t/sig '[(s/coll-of number? :kind vector?) :-> ::names]} [values]
  (numeric-labels number-label values))
