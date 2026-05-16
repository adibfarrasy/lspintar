package com.example;

public class JavaConsumer {
    private GroovyService groovyService;
    private KotlinService kotlinService;

    public String useGroovy(String input) {
        return groovyService.process(input);
    }

    public String useKotlin(String input) {
        return kotlinService.process(input);
    }
}
