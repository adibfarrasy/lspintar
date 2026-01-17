package com.example.app

import com.example.api.UserController

class Application {
    static void main(String[] args) {
        def userController = new UserController()
        userController.execute()
        userController.process([name: 'John', age: 30])
    }
}
