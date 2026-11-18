(ns demo.jvm-forms
  (:require [clojure.spec.alpha :as s]))

(set! *warn-on-reflection* true)

(definterface TextSource
  (^String text []))

(deftype TextBox [^String value]
  TextSource
  (text [_] value))

(defprotocol Ready
  (ready [box]))

(extend TextBox Ready {:ready (constantly :ready)})

(comment (println "A development-only expression"))

(defn display-name {:t/sig '[{:name (s/nilable string?)} :-> string?]}
  [options]
  (or (:name options) "Untitled"))
