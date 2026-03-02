package com.example

class ControllerHelper {
    fun demoValVar(repository: UserRepository) {
        // hover on user  -> User
        val user = repository.findById(1L)

        // hover on users -> List<User>
        val users = repository.findAll()

        // hover on it    -> User
        // hover on name  -> String
        val names = users.map { it.name }

        // var: hover on count -> Long
        var count = repository.count()

        // hover on it    -> User
        val adults = users.filter { it.age > 18 }

        // chained: first it -> User, second it -> String
        val trimmed = users.map { it.name }.map { it.trim() }
    }
}
