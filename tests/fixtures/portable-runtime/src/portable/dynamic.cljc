(ns portable.dynamic)

(def ^:dynamic *mode* :root)

(defn mode []
  *mode*)
