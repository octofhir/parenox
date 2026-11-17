(ns demo.handlers-bad)

(defn invalid-recovery {:t/sig '[:-> number?]} []
  (try
    (throw (ex-info "Missing request id" {:field :id}))
    (catch ExceptionInfo e
      (inc (:field (ex-data e))))))

(defn invalid-transfer {:t/sig '[:-> number?]} []
  (loop [i 0]
    (try (recur (inc i))
         (finally nil))))
