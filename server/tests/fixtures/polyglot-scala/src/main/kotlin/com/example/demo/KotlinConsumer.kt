package com.example

class KotlinConsumer {
    lateinit var groovyService: GroovyService

    fun useGroovy(input: String): String = groovyService.process(input)
}
