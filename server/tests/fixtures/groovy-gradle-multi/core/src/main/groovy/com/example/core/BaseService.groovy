package com.example.core

import java.io.Serializable

abstract class BaseService implements Serializable {
    private static final long serialVersionUID = 1L
    
    abstract void execute()
    
    void log(String message) {
        println "[${this.class.simpleName}] $message"
    }
}
