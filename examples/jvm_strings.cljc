(ns demo.jvm-strings
  (:require [clojure.spec.alpha :as s]))

(defn supplementary-length
  {:t/sig '[:-> #{"pair"}]}
  []
  (case (.length "🙂")
    2 "pair"
    "other"))

(defn maybe-code-unit
  {:t/sig '[(t/map {:opt {"name" (s/nilable string?)}}) number?
            :-> (s/nilable number?)]}
  [{:strs [name]} index]
  (when name
    (long ^Character (. ^String name (charAt index)))))
