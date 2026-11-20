#?(:clj (ns portable.jvm-launcher)
   :rust (ns portable.launcher
           (:require [clojure.core.async :as async]))
   :cljrs (ns portable.inactive-launcher))

#?(:rust
   (defn ^:async -main [& args]
     (let [result (await (async/go [(slurp "input/message.txt")
                                   (slurp "/dev/stdin")]))]
       (println "async main completed")
       (prn [result (vec args)])
       (when (= "--fail" (first args))
         (throw (ex-info "requested native main failure" {:kind :fixture})))))
   :cljrs (def launch-mode :inactive))
