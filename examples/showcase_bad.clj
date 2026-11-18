(ns demo.showcase-bad
  (:require [clojure.spec.alpha :as s]
            [demo.showcase :as batch]))

;; A release accidentally drops support for text notes. The input contract
;; still promises that both meter readings and notes are accepted.
(defn event-row {:t/sig '[::batch/event number? :-> {:text string? :score number?}]}
  [event weight]
  (case (:kind event)
    :meter {:text "Meter reading" :score (* (:value event) weight)}))

;; The feature flag alone says nothing about whether a title was supplied.
(defn use-title?
  {:t/sig '[boolean? (s/nilable string?) :-> boolean?]
   :t/guard '[:then title string?]}
  [enabled title]
  enabled)

(defn heading {:t/sig '[::batch/labels boolean? :-> string?]} [labels enabled]
  (let [title (batch/find-label labels :title)]
    (if (use-title? enabled title)
      (batch/clean-label title)
      "Default batch")))

;; Each vector contributes a distinct argument type to the named callback.
(defn zip-rows
  {:t/sig '[::batch/events ::batch/weights
            :-> (s/coll-of {:text string? :score number?} :kind vector?)]}
  [events weights]
  (batch/zip-results batch/event-row weights events))

;; A formatting change turns the numeric field of an API response into text.
(defn respond-total
  {:t/sig '[number? :-> {:status number? :body {:score number?}}]}
  [total]
  (batch/respond (fn [score] {:body {:score (str score)}}) {:payload total}))

;; The handler exposes an internal error number in a field meant for readers.
(defn invalid-tenant {:t/sig '[:-> {:status number? :body {:title string?}}]} []
  (try
    (throw (ex-info "Tenant missing" {:reason 422}))
    (catch clojure.lang.ExceptionInfo failure
      {:status 422 :body {:title (:reason (ex-data failure))}})))
