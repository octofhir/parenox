(println "standard output")
(binding [*out* *err*]
  (println "redirected output"))
(prn :streams)
