(ns demo.guards
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def ::patient (s/keys :opt-un [:patient/name]))

(defn display-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (if-let [name (:name patient)]
    (str/upper-case name)
    "UNKNOWN"))

(defn maybe-label
  {:t/sig '[(s/nilable ::patient) :-> (s/nilable string?)]}
  [patient]
  (when-let [{:keys [name]} patient]
    (str/upper-case (or name "Unknown"))))

(defn status
  {:t/sig '[(s/nilable boolean?) :-> string?]}
  [active?]
  (if-some [value active?]
    (if value "active" "inactive")
    "unknown"))

(defn maybe-status
  {:t/sig '[(s/nilable boolean?) :-> (s/nilable string?)]}
  [active?]
  (when-some [value active?]
    (if value "active" "inactive")))
