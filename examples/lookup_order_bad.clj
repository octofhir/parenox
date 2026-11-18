(ns demo.lookup-order-bad)

(defn wrong-recovery {:t/sig '[:-> #{:ready}]} []
  (try
    (get-in {:request {:status :ready}} [:request :status]
            (throw (ex-info "Default failed" {:stage :default})))
    (catch ExceptionInfo e (:stage (ex-data e)))))
