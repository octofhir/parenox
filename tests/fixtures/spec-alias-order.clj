(ns demo.spec-alias-order
  (:require [clojure.spec.alpha :as s]))

;; A fresh alias must resolve its target before it can enter the registry.
(let [before (s/registry)
      error (try
              (s/def ::early ::target)
              nil
              (catch Exception error error))]
  (assert (= Exception (class error)))
  (assert (= (str "Unable to resolve spec: " ::target) (.getMessage error)))
  (assert (identical? before (s/registry)))
  (assert (nil? (s/get-spec ::early)))
  (assert (nil? (s/get-spec ::target))))

(s/def ::target string?)
(assert (nil? (s/get-spec ::early)))
(assert (= ::early (s/def ::early ::target)))
(assert (= ::second-link (s/def ::second-link ::early)))
(assert (= ::third-link (s/def ::third-link ::second-link)))
(let [text (String. "registered")]
  (doseq [spec [::target ::early ::second-link ::third-link]]
    (assert (s/valid? spec text))
    (assert (identical? text (s/conform spec text)))
    (assert (= ::s/invalid (s/conform spec nil)))))

;; A map spec can name a key before that key's value spec is registered.
(assert (= ::row (s/def ::row (s/keys :req-un [::payload]))))
(assert (nil? (s/get-spec ::payload)))
(assert (s/valid? ::row {:payload nil}))
(assert (not (s/valid? ::row {})))
(s/def ::payload string?)
(let [input {:payload (String. "ready")}]
  (assert (s/valid? ::row input))
  (assert (identical? input (s/conform ::row input))))
(assert (= ::s/invalid (s/conform ::row {:payload nil})))
(assert (= ::s/invalid (s/conform ::row {})))

(prn :spec-alias-order-ok (clojure-version))
