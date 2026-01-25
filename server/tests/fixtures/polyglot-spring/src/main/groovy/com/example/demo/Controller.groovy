package com.example

import org.springframework.beans.factory.annotation.Autowired
import org.springframework.stereotype.Component
import org.apache.commons.lang3.StringUtils

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
        // Cursor on StringUtils -> should go to external dependency static class
        // Cursor on capitalize -> should go to external dependency StringUtil.capitalize()
        String input = StringUtils.capitalize("input")

        // Cursor on process -> should go to JavaService.process() (Java)
        String javaResult = javaService.process(input)
        
        // Cursor on process -> should go to GroovyService.process() (Groovy)
        String groovyResult = groovyService.process(input)
        
        // Cursor on process -> should go to KotlinService.process() (Kotlin)
        String kotlinResult = kotlinService.process(input)
        
        // Cursor on findById -> should go to UserRepository.findById() (Kotlin impl)
        User user = userRepository.findById(1L)
        
        // Cursor on save -> should go to UserRepository.save() (Kotlin impl)
        userRepository.save(user)
        
        // Cursor on name -> should go to User.name (Kotlin data class)
        String userName = user.name
    }
}
