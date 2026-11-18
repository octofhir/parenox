(ns vendor.clinic
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn find-patient {:t/sig '[string? :-> (s/nilable string?)]} [id]
  (if (str/starts-with? id "p-") "Ada" nil))

(defn render-label {:t/sig '[(s/nilable string?) :-> string?]} [patient]
  (or patient "Unknown"))
