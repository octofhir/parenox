(ns bad-title
  (:require [context-web-pg.handler :as handler]))

(defn bad-search {:t/sig '[map? :-> :context-web-pg.handler/response]} [ctx]
  (handler/search ctx 42))
