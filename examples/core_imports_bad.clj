(ns demo.core-imports-bad
  (:refer-clojure :only [defn number?]))

(defn advance {:t/sig '[number? :-> number?]} [value]
  (inc value))
