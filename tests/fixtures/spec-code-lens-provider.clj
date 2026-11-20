(ns spec-code-lens-provider
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-code-lens-provider.clj
;; lsp4clj 2.0.1 uses this conformer in its server capabilities spec.
(s/def :server-capabilities/code-lens-provider
  (s/conformer (fn [element] {:resolve-provider element})))
(s/def ::server-capabilities
  (s/keys :opt-un [:server-capabilities/code-lens-provider]))

(doseq [element [nil false ::s/invalid :enabled {:enabled true}]]
  (let [result (s/conform :server-capabilities/code-lens-provider element)
        capabilities {:code-lens-provider element}]
    (assert (= {:resolve-provider element} result))
    (assert (identical? element (:resolve-provider result)))
    (assert (true? (s/valid? :server-capabilities/code-lens-provider element)))
    (assert (= {:code-lens-provider result} (s/conform ::server-capabilities capabilities)))
    (assert (true? (s/valid? ::server-capabilities capabilities)))))

(prn {:clojure (clojure-version) :spec-code-lens-provider :ok})
