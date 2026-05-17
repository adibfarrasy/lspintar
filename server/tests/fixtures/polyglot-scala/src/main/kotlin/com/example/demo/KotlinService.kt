package com.example

import org.springframework.stereotype.Service

@Service
class KotlinService {
    fun process(input: String): String = "Kotlin: $input"
}
