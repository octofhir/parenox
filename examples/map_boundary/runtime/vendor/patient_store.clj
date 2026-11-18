(ns vendor.patient-store)

;; The JVM smoke test uses a deterministic stand-in, not a database.
(defn fetch [id]
  (case id
    "p-1" {:row {:display-name "Ada"}}
    "p-2" {:row {}}
    "p-3" {:row {:display-name nil}}
    nil))
