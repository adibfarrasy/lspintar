package com.example

import java.io.Serializable;

class UserService extends BaseService implements Serializable {
    private Repository repo

    @Getter
    @Setter
    private String userVariable
    
    @CompileDynamic
    User findUser(String id) {
        return repo.find(id)
    }

    @Override
    void execute() {
        println("Executing user service")
    }
}
