(ns java-import-completion
  (:import [java.util UUID]))

(defn random-id []
  (UUID/randomUUID))

(defn id-text [id]
  (UUID/.toString id))

(defn high [x y]
  (Math/max x y))

(def circle-constant Math/PI)
