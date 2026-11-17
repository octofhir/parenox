(ns demo.core-imports
  (:refer-clojure :exclude [inc] :rename {dec previous}))

(defn inc {:t/sig '[string? :-> number?]} [label]
  (clojure.core/inc (count label)))

(defn label-width {:t/sig '[string? :-> number?]} [label]
  (inc label))

(defn prior {:t/sig '[number? :-> number?]} [value]
  (previous value))
