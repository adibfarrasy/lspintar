package com.example

import org.springframework.stereotype.Service
import groovy.transform.CompileStatic

@CompileStatic
@Service
class GroovyService {
    String process(String input) {
        "Groovy: $input"
    }
}
