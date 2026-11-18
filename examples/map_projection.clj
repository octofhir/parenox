(ns demo.map-projection (:require [clojure.spec.alpha :as s]))

;; The stored record is open. The response exposes only the declared public fields.
(defn public-profile
  {:t/sig '[(t/map {:req {:id string? :enabled boolean? :internal-token string?}
                    :opt {"display" (s/nilable string?)}
                    :extra :open})
            :-> (t/map {:req {:id string? :enabled boolean?}
                        :opt {"display" (s/nilable string?)}})]}
  [profile]
  (let [public-fields [:id :enabled "display"]]
    (select-keys profile public-fields)))

;; Other fields can pass through. The contract explicitly excludes the token.
(defn redact-profile
  {:t/sig '[(t/map {:req {:id string? :internal-token string?} :extra :open})
            :-> (t/map {:req {:id string?}
                        :opt {:internal-token #{}}
                        :extra :open})]}
  [profile]
  (dissoc profile :internal-token))
