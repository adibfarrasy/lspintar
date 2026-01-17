package com.example

import groovy.transform.CompileDynamic

import java.io.Serializable;

class UserService extends BaseService implements Serializable {
    private Repository repo

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
