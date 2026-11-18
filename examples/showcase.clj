(ns demo.showcase
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

;; A batch contains either numeric meter readings or text notes. Both use
;; :value, but its type depends on the event's :kind.
(s/def :meter/kind #{:meter})
(s/def :meter/value number?)
(s/def :note/kind #{:note})
(s/def :note/value string?)
(s/def ::meter (s/keys :req-un [:meter/kind :meter/value]))
(s/def ::note (s/keys :req-un [:note/kind :note/value]))
(s/def ::event (s/or :meter ::meter :note ::note))
(s/def ::events (s/coll-of ::event :kind vector?))
(s/def ::weights (s/coll-of number? :kind vector?))
(s/def ::labels (s/map-of keyword? (s/nilable string?)))
(s/def ::custom-title? boolean?)
(s/def ::batch (s/keys :req-un [::events ::weights ::labels ::custom-title?]))

;; Two input parameters come from the vectors. The named callback determines
;; the result type. mapv stops at the end of the shorter vector.
(defn zip-results
  {:t/sig '(t/all [a b c]
             [(s/fspec :args (s/cat :event a :weight b) :ret c)
              (s/coll-of a :kind vector?) (s/coll-of b :kind vector?)
              :-> (s/coll-of c :kind vector?)])}
  [render events weights]
  (mapv render events weights))

(defn find-label
  {:t/sig '(t/all [a] [(s/map-of keyword? a) keyword? :-> (s/nilable a)])}
  [labels key]
  (get labels key))

;; Parenox verifies this implication from the body before it narrows callers.
;; Replace the body with enabled: the guard no longer proves title is a string.
(defn use-title?
  {:t/sig '[boolean? (s/nilable string?) :-> boolean?]
   :t/guard '[:then title string?]}
  [enabled title]
  (and enabled (string? title)))

(s/fdef clean-label :args (s/cat :text string?) :ret string?)
(defn clean-label [text]
  (-> text str/trim str/upper-case))

(defn event-row
  {:t/sig '[::event number? :-> {:text string? :score number?}]}
  [event weight]
  (let [kind (:kind event)]
    ;; Remove the :note clause to see the checker report the missing tag.
    (case kind
      :meter {:text "Meter reading" :score (* (:value event) weight)}
      :note {:text (clean-label (:value event)) :score 0})))

;; The two type parameters appear in required map fields, including the
;; callback result. The envelope adds status without erasing the body type.
(defn respond
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :payload a) :ret {:body b})
                        {:payload a} :-> {:status number? :body b}])}
  [render input]
  (assoc (render (:payload input)) :status 200))

(defn- require-tenant [labels]
  (if-some [tenant (get labels :tenant)]
    tenant
    (throw (ex-info "Tenant missing" {:reason "Tenant is required"}))))

(defn- tenant-result [labels]
  (try
    {:kind :ready :value (require-tenant labels)}
    (catch clojure.lang.ExceptionInfo failure
      {:kind :invalid :value (:reason (ex-data failure))})))

;; Private helpers can get their types from a typed caller.
(defn- score-total [rows]
  (reduce (fn [total row] (+ total (:score row))) 0 rows))

(defn summarize-batch
  {:t/sig '[::batch :-> {:status number?
                        :body {:title string?
                               :score number?
                               :rows (s/coll-of {:text string? :score number?} :kind vector?)}}]}
  [{:keys! [events weights labels custom-title?]}]
  (let [tenant (tenant-result labels)]
    (case (:kind tenant)
      :invalid {:status 422 :body {:title (:value tenant) :score 0 :rows []}}
      :ready
      (let [title (find-label labels :title)
            heading (if (use-title? custom-title? title)
                      (clean-label title)
                      (:value tenant))
            rows (zip-results event-row events weights)]
        (respond (fn [entries]
                   {:body {:title heading :score (score-total entries) :rows entries}})
                 {:payload rows})))))

(defn sample-batch {:t/sig '[:-> ::batch]} []
  {:events [{:kind :meter :value 7} {:kind :note :value "  check  "}]
   :weights [2 10]
   :labels {:tenant "North hub" :title "  shift a  "}
   :custom-title? true})
