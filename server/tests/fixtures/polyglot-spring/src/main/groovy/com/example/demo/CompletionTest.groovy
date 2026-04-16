package com.example.demo

// Deliberately no explicit imports to exercise implicit import resolution

class CompletionTest {
    GroovyService groovyService

    void testPrefixExcludesMembers() {
        // cursor at end of "capitalize" on next line – StringUtils#capitalize must not appear
        capitalize
    }

    void testLocalsBeforeGlobals() {
        // groovyResult (local) must appear before GroovyService (global) for prefix "groovy"
        String groovyResult = "test"
        groovy
    }

    void testImplicitImportChain() {
        // Closure is in groovy.lang.* – an implicit import – no explicit import needed
        Closure myClosure = {}
        myClosure.
    }
}
