package com.example.core

import java.util.concurrent.Callable

interface DataProcessor extends Callable<String> {
    static final int MAX_BATCH_SIZE = 1000

    void process(Map<String, Object> data)

    static List<String> processInBatches(List<String> items) {
            // implementation
            return items
        }
}
