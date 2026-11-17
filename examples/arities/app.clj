(ns demo.arities.app
  (:require [demo.arities.provider :as provider]))

(defn next-offset {:t/sig '[number? :-> number?]} [offset]
  (provider/count-offset offset))

(defn label-size {:t/sig '[string? :-> number?]} [label]
  (let [describe provider/describe]
    (describe label)))

(defn ready-label {:t/sig '[:-> string?]} []
  (provider/describe))
