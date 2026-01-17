package com.example

import groovy.transform.CompileStatic

@CompileStatic
class User {
    String name
    int age
    
    String getDisplayName() {
        return name
    }
}
