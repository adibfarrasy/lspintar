package com.example

import org.springframework.stereotype.Service

@Service
class ScalaService {
  def process(input: String): String = s"Scala: $input"

  def compute(n: Int): Int = n * 2
}
