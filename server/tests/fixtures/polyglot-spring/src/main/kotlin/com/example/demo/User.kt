package com.example

data class User(
    val id: Long,
    val name: String,
    val status: String,
    val occupation: String = "unemployed",
)
