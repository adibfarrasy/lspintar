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
    void process(Map<String, Object> data) {
        process(data)
    }

    @Override
    ApiResponse process(Map<String, Object> data) {
        log("Processing user data: $data")
    }

    void demoGoToDefinition() {
        // Strategy 1: Static member access
        // Cursor on MAX_BATCH_SIZE -> should go to DataProcessor.MAX_BATCH_SIZE
        int maxSize = DataProcessor.MAX_BATCH_SIZE
        
        // Cursor on processInBatches -> should go to DataProcessor.processInBatches()
        def items = DataProcessor.processInBatches(['a', 'b', 'c'])
        
        // Strategy 2: "this" qualifier
        // Cursor on execute -> should go to UserController.execute()
        this.execute()
        
        // Cursor on serviceName -> should go to BaseService.serviceName field (inherited)
        this.serviceName = "user-api"
        
        // Cursor on log -> should go to BaseService.log() (inherited method)
        this.log("test message")
        
        // Strategy 3: Instance member access (variable)
        UserController controller = new UserController()
        
        // Cursor on process -> should go to UserController.process()
        ApiResponse response = controller.process([key: 'value'])
        
        // Cursor on status -> should go to ApiResponse.status field
        String status = response.status
        
        // Strategy 4: Chained calls (complex)
        // Cursor on getEnvironment -> should go to DataConfig.getEnvironment()
        // Requires: resolve controller type -> UserController,
        //           resolve getConfig() return type -> DataConfig (from BaseService)
        String env = controller.getConfig().getEnvironment()
        
        // Strategy 5: Fallback to qualifier
        // If someUnknownMethod doesn't exist in database:
        // Cursor on someUnknownMethod -> should at least jump to UserController definition
        controller.someUnknownMethod()
    }
}
