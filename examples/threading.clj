(ns demo.threading
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def ::patient (s/keys :opt-un [:patient/name]))

(defn display-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (-> patient :name (or "Unknown") str/trim str/upper-case))

(defn numbered
  {:t/sig '[number? string? :-> string?]}
  [number name]
  (str number ": " name))

(defn numbered-name
  {:t/sig '[string? :-> string?]}
  [name]
  (->> name (numbered 7)))

(defn maybe-name
  {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]}
  [name]
  (some-> name str/trim str/upper-case))

(defn maybe-numbered
  {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]}
  [name]
  (some->> name (numbered 7)))

(defn false-text
  {:t/sig '[:-> string?]}
  []
  (some-> false str))

(defn choose-label
  {:t/sig '[boolean? :-> string?]}
  [use-fallback?]
  (->> (throw (ex-info "Name required" {:stage :input}))
       (if use-fallback? "Untitled")))
