(ns demo.literal-vars.config)

(def ^:const default-plan {:limit 4 :mode ::online})
(def status "Response status." :active)
(def ^:private fallback "Unknown")

(defn fallback-name {:t/sig '[:-> string?]} []
  fallback)
