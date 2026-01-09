package com.example

class UserService {
    private Repository repo

    @Getter
    @Setter
    private String userVariable
    
    @CompileDynamic
    User findUser(String id) {
        return repo.find(id)
    }
}
