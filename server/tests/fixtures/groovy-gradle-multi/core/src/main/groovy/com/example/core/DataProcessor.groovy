package com.example.core

import java.util.concurrent.Callable

interface DataProcessor {
    static final int MAX_BATCH_SIZE = 1000

    DataProcessResult process(Map<String, Object> data)

}
