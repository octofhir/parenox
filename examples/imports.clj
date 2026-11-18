(ns demo.imports
  (:import (java.io IOException FileNotFoundException)
           [clojure.lang ExceptionInfo]
           java.lang.ArithmeticException))

(defn request-stage {:t/sig '[:-> #{:request}]} []
  (try
    (throw (ex-info "Missing request" {:stage :request}))
    (catch FileNotFoundException _ :file)
    (catch IOException _ :io)
    (catch ArithmeticException _ :arithmetic)
    (catch ExceptionInfo e (:stage (ex-data e)))))
