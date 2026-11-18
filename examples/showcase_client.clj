(ns demo.showcase-client
  (:require [demo.showcase :as batch]))

;; Parenox can check this against the exported EDN alone.
(defn render {:t/sig '[:-> string?]} []
  (let [sample (batch/sample-batch)]
    (:title (:body (batch/summarize-batch (assoc sample :trace-id "run-42"))))))
