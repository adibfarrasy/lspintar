package com.example.api

import com.example.core.BaseService
import com.example.core.DataProcessor
import com.example.core.DataProcessResult

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
    DataProcessResult process(Map<String, Object> data) {
        processInternal(data, "default")
    }

    private void processInternal(Map<String, Object> data, String label) {
        log("Processing user data: $data [$label]")
    }
    
    private void processInternal(Map<String, Object> data, String label, int priority) {
        log("Processing user data: $data [$label] with priority $priority")
    }

    private void processInternal(List<String> data) {
        log("Processing user data: $data")
    }

    private void processInternal(Map<String, Object> data) {
        log("Processing user data: $data")
    }

    void demoGoToDefinition() {
        // 1: Static member access
        // Cursor on MAX_BATCH_SIZE -> should go to DataProcessor.MAX_BATCH_SIZE
        int maxSize = DataProcessor.MAX_BATCH_SIZE
        
        // Cursor on processInBatches -> should go to DataProcessor.processInBatches()
        def items = DataProcessor.processInBatches(['a', 'b', 'c'])
        
        // 2: "this" qualifier
        // Cursor on execute -> should go to UserController.execute()
        this.execute()
        
        // Cursor on serviceName -> should go to BaseService.serviceName field (inherited)
        this.serviceName = "user-api"
        
        // Cursor on log -> should go to BaseService.log() (inherited method)
        this.log("test message")
        
        // 3: Instance member access (variable)
        UserController controller = new UserController()
        
        // Cursor on process -> should go to UserController.process()
        DataProcessResult response = controller.process([key: 'value'])
        
        // Cursor on status -> should go to ApiResponse.status field
        String status = response.status
        
        // 4: Chained calls
        controller.process([key: 'value']).message

        new UserController().process([key: 'value']).message

         // Method overloading - different arity
        this.processInternal([user: 'john'], "test")
        
        this.processInternal([user: 'jane'], "urgent", 1)

        // Method overloading - different parameter types, same arity
        this.processInternal(['john', 'jane'])
        
        this.processInternal([user: 'john'])
    }
}
