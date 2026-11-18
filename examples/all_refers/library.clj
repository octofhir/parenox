(ns demo.all-refers.library
  (:refer-clojure :exclude [inc]))

(def limit 7)

(defn label {:t/sig '[:-> string?]} []
  "ready")

(defn inc {:t/sig '[number? :-> number?]} [n]
  (+ n 10))

(defn skipped {:t/sig '[:-> number?]} []
  4)

(defn- hidden {:t/sig '[:-> string?]} []
  "private")
