(ns demo.exceptions
  (:require [clojure.spec.alpha :as s]))

(defn required-id {:t/sig '[(s/nilable string?) :-> string?]} [id]
  (if-some [present id]
    present
    (throw (ex-info "Missing request id" {:field :id :code "MISSING"}))))

(defn error-code {:t/sig '[:-> string?]} []
  (:code (ex-data (ex-info nil {:code "MISSING"}))))

(defn absent-code {:t/sig '[:-> string?]} []
  (:code (ex-data (ex-info "No details" nil)) "unknown"))

(defn stop-before-later-arguments {:t/sig '[:-> string?]} []
  (+ 1 (throw (ex-info "Stopped" {})) (inc "unreachable")))
