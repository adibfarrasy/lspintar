package com.example

import org.springframework.stereotype.Repository

@Repository
class UserRepository : BaseRepository<User> {
    override fun findById(id: Long): User {
        return User(id, "User $id")
    }
    
    override fun save(entity: User) {
        println("Saving: ${entity.name}")
    }
}
