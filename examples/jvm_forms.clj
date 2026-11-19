(ns demo.jvm-forms
  (:require [clojure.spec.alpha :as s]))

(set! *warn-on-reflection* true)
(set! *unchecked-math* false)

(definterface TextSource
  (^String text []))

(deftype TextBox [^String value]
  TextSource
  (text [_] value))

(defprotocol Ready
  (ready [box]))

(extend-type TextBox Ready (ready [_] :ready))

(extend-protocol Ready nil (ready [_] :absent))

(comment (println "A development-only expression"))

(defonce ^:private counter (atom 0))

(declare render)

(defmulti render "Render a value." type)
(defmethod render String [text] (str (swap! counter inc) ":" text))

(defn display-name {:t/sig '[{:name (s/nilable string?)} :-> string?]}
  [options]
  (or (:name options) "Untitled"))
