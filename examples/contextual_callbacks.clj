(ns demo.contextual-callbacks
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def ::text-transform
  (s/fspec :args (s/cat :text string?) :ret string?))

(defn transform {:t/sig '[::text-transform string? :-> string?]} [f text]
  (f text))

(defn upper-label {:t/sig '[string? :-> string?]} [label]
  (transform (fn [text] (str/upper-case text)) label))

(defn wrapped-label {:t/sig '[string? string? :-> string?]} [label separator]
  (transform #(str separator (str/upper-case %) separator) label))
