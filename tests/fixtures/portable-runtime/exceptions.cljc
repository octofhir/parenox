(try
  (try
    (throw (ex-info "portable original" {:code :original}))
    (catch #?(:clj clojure.lang.ExceptionInfo :cljrs Exception) error
      (println "caught original")
      (throw (ex-info "portable wrapper" {:seen (ex-data error)})))
    (finally
      (println "ran finally")))
  (catch #?(:clj clojure.lang.ExceptionInfo :cljrs Exception) error
    (prn [(ex-message error) (ex-data error)])))
