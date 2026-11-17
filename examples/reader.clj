(ns demo.reader
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def ::separator #{\space \-})

;; Metadata stays on the form. A JVM type hint is not a type contract.
(defn ^:private normalize [^String name]
  (str/upper-case (or name "Unknown")))

(defn ^{:t/sig '[(s/nilable string?) char? :-> string?]} label
  [name separator]
  (str (normalize name) separator))

(defn default-separator
  {:t/sig '[:-> ::separator]}
  []
  \-)
