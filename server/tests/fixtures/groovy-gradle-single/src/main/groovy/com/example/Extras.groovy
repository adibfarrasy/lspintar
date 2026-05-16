package com.example

import groovy.transform.CompileStatic

@CompileStatic
class Extras {
    static final int MAX_RETRIES = 3

    private final List<String> tags = []

    static String describe() {
        return "extras"
    }

    Pair<String, Integer> tagged(String name, int count) {
        return new Pair<String, Integer>(name, count)
    }

    static class Pair<A, B> {
        A first
        B second

        Pair(A a, B b) {
            this.first = a
            this.second = b
        }
    }

    enum Severity {
        LOW, MEDIUM, HIGH
    }
}
