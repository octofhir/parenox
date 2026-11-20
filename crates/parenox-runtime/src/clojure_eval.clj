(ns parenox.runtime.clojure-eval
  (:import (clojure.lang LineNumberingPushbackReader)
           (java.io StringReader)))

(defn- evaluate-source [previous ^String source]
  (with-open [reader (LineNumberingPushbackReader. (StringReader. source))]
    (let [eof (Object.)]
      (loop [result previous]
        (let [form (read {:eof eof} reader)]
          (if (identical? form eof)
            result
            (recur (eval form))))))))

(defn- failure [error]
  (let [causes (take 64 (take-while some? (iterate #(.getCause ^Throwable %) error)))
        cause (last causes)
        exception (.getName (class cause))
        message (ex-message cause)]
    [:run-error :execution (or message exception) nil nil nil
     [exception message]]))

(let [[result & expressions] *command-line-args*]
  (spit result ":started" :encoding "UTF-8")
  (let [report (try
                 (binding [*ns* (the-ns 'user) *file* "<expr>" *command-line-args* nil]
                   [:value (pr-str (reduce evaluate-source nil expressions))])
                 (catch Throwable error (failure error)))]
    (spit result (pr-str report) :encoding "UTF-8")))

(System/exit 0)
