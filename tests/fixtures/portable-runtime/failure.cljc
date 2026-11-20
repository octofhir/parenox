(println "before exception")
(throw (ex-info "portable failure" {:expected true}))
