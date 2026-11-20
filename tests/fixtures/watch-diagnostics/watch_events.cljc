(ns watch-events (:require [lsp4clj.coercer :as coercer]))

(defn watched-files
  {:t/sig '[:lsp4clj.coercer/did-change-watched-files-params :-> number?]}
  [params]
  1)

(defn watch-event {:t/sig '[:-> number?]} []
  (watched-files
    {:changes [{:uri "file:///first.clj" :type 1}
               {:uri "file:///🦊.clj" :type #_9 2}]}))
