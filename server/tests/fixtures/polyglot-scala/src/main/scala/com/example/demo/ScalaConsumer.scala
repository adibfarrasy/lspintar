package com.example

class ScalaConsumer {
  var javaService: JavaService = _
  var kotlinService: KotlinService = _
  var groovyService: GroovyService = _

  def useJava(input: String): String = javaService.process(input)

  def useKotlin(input: String): String = kotlinService.process(input)

  def useGroovy(input: String): String = groovyService.process(input)
}
