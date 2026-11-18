(ns demo.lookup-order)

(defn eager-default {:t/sig '[:-> #{:default}]} []
  (try
    (get {:status :ready} :status
         (throw (ex-info "Default failed" {:stage :default})))
    (catch ExceptionInfo e (:stage (ex-data e)))))

(defn first-path-error {:t/sig '[:-> #{:key}]} []
  (try
    (get-in {}
            [(throw (ex-info "Key failed" {:stage :key}))
             (throw (ex-info "Later key failed" {:stage :later-key}))]
            (throw (ex-info "Default failed" {:stage :default})))
    (catch ExceptionInfo e (:stage (ex-data e)))))

(defn scalar-fallback {:t/sig '[:-> #{:missing}]} []
  (try
    (get 42 :status :missing)
    (catch ExceptionInfo _ :exception)))

(defn string-index-present? {:t/sig '[:-> #{true}]} []
  (contains? "😀" 1))

(defn rejected-write {:t/sig '[:-> #{:value}]} []
  (try
    (assoc-in {:request {:status :ready}} [:request :status]
              (throw (ex-info "Value failed" {:stage :value})))
    (catch ExceptionInfo e (:stage (ex-data e)))))

(defn saved-status {:t/sig '[:-> #{:ready}]} []
  (try
    (get (assoc {} :status :ready :attempt 1) :status)
    (catch Throwable _ :failed)))
