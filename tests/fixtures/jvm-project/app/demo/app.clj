(ns demo.app
  (:require [demo.greeting :as greeting]))

(defn -main [& args]
  (prn [(greeting/welcome (read-line)) (System/getProperty "user.dir") (vec args)])
  (binding [*out* *err*]
    (println "app diagnostic"))
  (System/exit 17))
