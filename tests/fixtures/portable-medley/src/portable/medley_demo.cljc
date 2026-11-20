(ns portable.medley-demo
  (:require [medley.core :as medley]))

(defn scenario []
  (let [merged (medley/deep-merge {:outer {:x 1}} {:outer {:y 2}})
        filtered (medley/assoc-some {} :missing nil :keep false :next 7)]
    [(medley/find-first odd? [2 4 5 7])
     [(get-in merged [:outer :x]) (get-in merged [:outer :y])]
     (medley/dissoc-in {:drop {:leaf 6} :keep 9} [:drop :leaf])
     [(:keep filtered) (:next filtered) (contains? filtered :missing)]]))

(defn -main [& args]
  (println "medley scenario λ")
  (prn [(vec args) (scenario)]))
