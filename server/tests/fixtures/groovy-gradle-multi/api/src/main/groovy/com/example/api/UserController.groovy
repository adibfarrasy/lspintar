package com.example.api

import com.example.core.BaseService
import com.example.core.DataProcessor

class UserController extends BaseService implements DataProcessor {
    
    private static class ApiResponse {
        String status
        int code
        String message
    }

    /**
    * lorem ipsum
    * dolor sit amet
    */
    @Override
    void execute() {
        log("Executing user controller")
    }
    
    @Override
    ApiResponse process(Map<String, Object> data) {
        log("Processing user data: $data")
    }
}
