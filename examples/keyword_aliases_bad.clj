(ns demo.keyword-aliases-bad)

(defn status-key {:t/sig '[:-> keyword?]} []
  #_ ::missing/status
  :ready)
