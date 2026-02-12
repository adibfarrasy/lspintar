package com.example

import groovy.transform.CompileDynamic
import groovy.json.JsonBuilder
import com.test.Dummy

import java.io.Serializable

class UserService extends BaseService implements Serializable {
    private Repository repo

    private String userVariable

    private Dummy dummy
    
    @CompileDynamic
    User findUser(String id) {
        return repo.find(id)
    }

    @Override
    void execute() {
        println("Executing user service")

        def builder = new JsonBuilder()
        
        builder {
            service "UserService"
            status "Running"
            config {
                retries 3
                timeout "5s"
            }
        }
        
        println "Current State: ${builder.toString()}"
    }
}
