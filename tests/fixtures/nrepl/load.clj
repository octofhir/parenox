(ns parenox-nrepl-fixture)

(println "loaded λ")
(binding [*out* *err*]
  (println "load warning"))

(defn score [] 40)
:loaded
