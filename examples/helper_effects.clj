(ns demo.helper-effects)

(defn- read-status [ready?]
  (if ready?
    :ready
    (throw (ex-info "Missing status" {:stage :missing}))))

(defn- constant [_] :constant)

(defn request-stage {:t/sig '[:-> #{:missing}]} []
  (try
    (read-status false)
    (catch ArithmeticException _ :arithmetic)
    (catch ExceptionInfo e (:stage (ex-data e)))))

(defn request-status {:t/sig '[:-> #{:ready}]} []
  (try
    (read-status true)
    (catch ExceptionInfo _ :unexpected)))

(defn rejected-argument {:t/sig '[:-> #{:param}]} []
  (try
    (constant (throw (ex-info "Parameter failed" {:stage :param})))
    (catch ExceptionInfo e (:stage (ex-data e)))))
