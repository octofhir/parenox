(ns context-web-pg.handler
  (:require [clojure.spec.alpha :as s]
            [context-web-pg.query :as query]))

(s/def ::status number?)
(s/def ::body :context-web-pg.query/rows)
(s/def ::response (s/keys :req-un [::status ::body]))

;; Ring-style response; the input title has already been validated.
(defn search
  {:t/sig '[map? string? :-> ::response]}
  [ctx title]
  {:status 200 :body (query/matching-rows ctx title)})
