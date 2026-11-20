(ns spec-conditional-provider
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-conditional-provider.clj
;; These conformers come from lsp4clj 2.0.1.
(s/def :server-capabilities/code-action-provider
  (s/conformer #(when (vector? %) {:code-action-kinds %})))
(s/def :server-capabilities/execute-command-provider
  (s/conformer #(when (vector? %) {:commands %})))
(s/def ::server-capabilities
  (s/keys :opt-un [:server-capabilities/code-action-provider
                   :server-capabilities/execute-command-provider]))

(doseq [[spec capability field]
        [[:server-capabilities/code-action-provider :code-action-provider :code-action-kinds]
         [:server-capabilities/execute-command-provider :execute-command-provider :commands]]
        [element wrapped?] [[nil false]
                            [false false]
                            [::s/invalid false]
                            [:enabled false]
                            [{:enabled true} false]
                            [(list :command) false]
                            [(java.util.ArrayList. [:command]) false]
                            [[] true]
                            [[nil false ::s/invalid] true]
                            [(subvec [:prefix :command] 1) true]
                            [(find {:entry :command} :entry) true]]]
  (let [result (s/conform spec element)
        expected (when wrapped? {field element})
        capabilities {capability element}]
    (assert (= expected result))
    (when wrapped?
      (assert (identical? element (get result field))))
    (assert (true? (s/valid? spec element)))
    (assert (= {capability expected} (s/conform ::server-capabilities capabilities)))
    (assert (true? (s/valid? ::server-capabilities capabilities)))))

(prn {:clojure (clojure-version) :spec-conditional-provider :ok})
