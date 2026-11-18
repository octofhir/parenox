(ns demo.helper-effects-bad)

(defn- missing []
  (throw (ex-info "Missing status" {:stage :missing})))

(defn request-stage {:t/sig '[:-> #{:ready}]} []
  (try
    (missing)
    (catch ExceptionInfo e (:stage (ex-data e)))))
