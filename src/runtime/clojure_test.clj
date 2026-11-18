(ns parenox.runtime.clojure-test
  (:require [clojure.java.io :as io]
            [clojure.string :as str]
            [clojure.test :as t]))

(defn- local-file [source]
  (let [file (io/file source)]
    (if (and (.isAbsolute file) (.isFile file))
      file
      (when-let [resource (io/resource source)]
        (when (= "file" (.getProtocol resource))
          (io/file resource))))))

(defn- position [root resolved line]
  (let [path (.toPath (.getCanonicalFile resolved))]
    [(if (.startsWith path root) (.relativize root path) path) line]))

(defn- test-location [root {:keys [type file line actual]} {:keys [ns] source :file}]
  (when (and ns source file (pos-int? line)
             (or (= file source) (= file (.getName (io/file source)))))
    (let [prefix (str (munge (str (ns-name ns))) "$")
          frames (if (and (= :error type) (instance? Throwable actual))
                   (.getStackTrace ^Throwable actual)
                   (.getStackTrace (Thread/currentThread)))]
      (when (some (fn [^StackTraceElement frame]
                    (and (= line (.getLineNumber frame))
                         (= file (.getFileName frame))
                         (str/starts-with? (.getClassName frame) prefix)))
                  frames)
        (when-let [resolved (local-file source)]
          (position root resolved line))))))

(defn- error-location [root ^Throwable error]
  (let [namespace-classes (into #{} (map (comp munge str ns-name)) (all-ns))]
    (some (fn [^StackTraceElement frame]
            (let [file (.getFileName frame)
                  line (.getLineNumber frame)
                  [prefix generated] (str/split (.getClassName frame) #"[$/]" 2)]
              (when (and generated (contains? namespace-classes prefix)
                         (pos-int? line) file
                         (or (str/ends-with? file ".clj") (str/ends-with? file ".cljc")))
                (let [package (subs prefix 0 (inc (.lastIndexOf ^String prefix ".")))
                      resource (str (str/replace package "." "/") file)]
                  (when-let [resolved (local-file resource)]
                    (position root resolved line))))))
          (.getStackTrace error))))

(defn- reporter [report]
  (let [root (.toPath (.getCanonicalFile (io/file ".")))]
    (fn [{:keys [type actual] :as event}]
      (when (#{:fail :error} type)
        (let [{:keys [ns name] :as metadata} (meta (first t/*testing-vars*))]
          (when-let [[file line] (or (when (and (= type :error) (instance? Throwable actual))
                                      (error-location root actual))
                                    (test-location root event metadata))]
            (t/with-test-out
              (println (str file ":" line ": " (clojure.core/name type)
                            (when ns (str " in " (ns-name ns) "/" name))))))))
      (report event))))

(let [[result & names] *command-line-args*
      namespaces (distinct (map symbol names))]
  (apply require namespaces)
  (let [summary (binding [t/report (reporter t/report)]
                  (apply t/run-tests namespaces))]
    (spit result (cond
                   (zero? (:test summary)) "empty"
                   (t/successful? summary) "pass"
                   :else "fail"))
    (shutdown-agents)))
