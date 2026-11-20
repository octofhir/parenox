(ns portable.summary
  (:require [portable.model :as model]))

(println "portable calculation")
(prn (conj (model/summarize 2 [1 2 3 4 5])
           #?(:clj :shared :cljrs :shared :default :unexpected)))
