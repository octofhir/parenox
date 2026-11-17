(ns demo.handlers
  (:require [clojure.spec.alpha :as s]))

(defn request-id {:t/sig '[(s/nilable string?) :-> string?]} [id]
  (try
    (if-some [present id]
      present
      (throw (ex-info "Missing request id" {:field :id})))
    (catch ExceptionInfo e
      (let [details (ex-data e)]
        (str "unknown-" (count details))))
    (finally nil)))

(defn guarded-count {:t/sig '[number? :-> number?]} [n]
  (try
    (loop [i 0 total 0]
      (if (= i n) total (recur (inc i) (+ total i))))
    (catch ArithmeticException _ 0)
    (finally 42)))

(defn cleanup-fails {:t/sig '[:-> string?]} []
  (try 7
       (finally (throw (ex-info "Cleanup failed" {})))))
