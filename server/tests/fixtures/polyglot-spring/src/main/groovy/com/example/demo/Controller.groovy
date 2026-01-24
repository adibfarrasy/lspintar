package com.example

import org.springframework.beans.factory.annotation.Autowired
import org.springframework.stereotype.Component

@Component
class Controller {
    
    @Autowired
    JavaService javaService
    
    @Autowired
    GroovyService groovyService
    
    @Autowired
    KotlinService kotlinService
    
    @Autowired
    UserRepository userRepository
    
    void demoGoToDefinition() {
        // 1: Cross-language service calls
        // Cursor on process -> should go to JavaService.process() (Java)
        String javaResult = javaService.process("test")
        
        // Cursor on process -> should go to GroovyService.process() (Groovy)
        String groovyResult = groovyService.process("test")
        
        // Cursor on process -> should go to KotlinService.process() (Kotlin)
        String kotlinResult = kotlinService.process("test")
        
        // 2: Interface implementation across languages
        // Cursor on findById -> should go to UserRepository.findById() (Kotlin impl)
        User user = userRepository.findById(1L)
        
        // Cursor on save -> should go to UserRepository.save() (Kotlin impl)
        userRepository.save(user)
        
        // Cursor on name -> should go to User.name (Kotlin data class)
        String userName = user.name
        
        // 3: Method chaining across languages
        String result = javaService.process(
            groovyService.process(
                kotlinService.process("input")
            )
        )
    }
}
