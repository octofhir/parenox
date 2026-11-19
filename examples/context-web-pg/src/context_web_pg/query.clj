(ns context-web-pg.query
  (:require [clojure.spec.alpha :as s]
            [pg]))

(s/def ::sql (s/coll-of string? :kind vector?))
(s/def ::titles-query (s/keys :req-un [::sql]))
(s/def ::title string?)
(s/def ::row (s/keys :req-un [::title]))
(s/def ::rows (s/coll-of map? :kind vector?))

(defn title-query
  {:t/sig '[string? :-> ::titles-query]}
  [title]
  {:sql ["SELECT title FROM todos WHERE title = ?" title]})

(defn matching-rows
  {:t/sig '[map? string? :-> ::rows]}
  [ctx title]
  (pg/execute! ctx (title-query title)))

(defn checked-row
  {:t/sig '[map? :-> ::row]}
  [row]
  (if (s/valid? ::row row)
    row
    (throw (ex-info "Database result has no string title" {}))))

;; Adapted from the loop in context-clj/pg's dsql.core/reduce-separated-old.
;; Only a known nonempty sequence enters the loop. The row fields are names,
;; not arbitrary SQL; this example never interpolates them into a query.
(defn projection-parts
  {:t/sig '[(s/coll-of string? :kind seq?) :-> (s/coll-of string? :kind vector?)]}
  [fields]
  (if (empty? fields)
    []
    (loop [[field & more] fields
           parts []]
      (let [parts (conj parts field)]
        (if (empty? more)
          parts
          (recur more (conj parts ",")))))))
