(require '[demo.routing :as routing])

(prn [(routing/status {:route :home}) (vec *command-line-args*)])
