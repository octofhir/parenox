(ns portable.selected-test
  (:require [clojure.test :refer [deftest is]]
            [portable.other-selected-test :refer [borrowed]]))

(deftest secret
  (println "selected body private")
  (is (:private (meta #'secret))))

(alter-meta! #'secret assoc :private true)

(deftest errors
  (println "selected body error")
  (throw (ex-info "selected error Δ" {:kind :fixture})))

(defn ordinary []
  (println "unexpected ordinary var"))

(deftest not-selected
  (println "unexpected unselected test")
  (is false))
