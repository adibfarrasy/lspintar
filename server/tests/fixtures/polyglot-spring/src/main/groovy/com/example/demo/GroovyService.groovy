package com.example

import org.springframework.stereotype.Service

@Service
class GroovyService {
    String process(String input) {
        "Groovy: $input"
    }
}
