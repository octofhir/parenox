(ns spec-signature-help-provider
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-signature-help-provider.clj
;; lsp4clj 2.0.1 tests vector? before map? in this conformer.
(s/def :server-capabilities/signature-help-provider
  (s/conformer #(cond (vector? %) {:trigger-characters %}
                      (map? %) %
                      :else {:trigger-characters %})))
(s/def ::server-capabilities
  (s/keys :opt-un [:server-capabilities/signature-help-provider]))

(let [spec :server-capabilities/signature-help-provider
      hybrid (proxy [Object clojure.lang.IPersistentMap clojure.lang.IPersistentVector] [])]
  (assert (map? hybrid))
  (assert (vector? hybrid))
  (doseq [element [nil false ::s/invalid :enabled
                   (java.util.HashMap. {:enabled true})
                   [] [nil false ::s/invalid] (subvec [:prefix :command] 1)
                   (find {:entry :command} :entry) hybrid]]
    (let [result (s/conform spec element)
          capabilities {:signature-help-provider element}
          conformed (:signature-help-provider (s/conform ::server-capabilities capabilities))]
      (assert (= [:trigger-characters] (keys result)))
      (assert (identical? element (:trigger-characters result)))
      (assert (true? (s/valid? spec element)))
      (assert (= [:trigger-characters] (keys conformed)))
      (assert (identical? element (:trigger-characters conformed)))
      (assert (true? (s/valid? ::server-capabilities capabilities)))))
  (doseq [element [{} {:trigger-characters ["."]} (sorted-map :marker ::s/invalid)]]
    (let [capabilities {:signature-help-provider element}]
      (assert (identical? element (s/conform spec element)))
      (assert (true? (s/valid? spec element)))
      (assert (identical? capabilities (s/conform ::server-capabilities capabilities)))
      (assert (true? (s/valid? ::server-capabilities capabilities))))))

(prn {:clojure (clojure-version) :spec-signature-help-provider :ok})
