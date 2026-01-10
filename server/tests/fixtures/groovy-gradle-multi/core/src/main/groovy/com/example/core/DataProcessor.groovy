package com.example.core

import java.util.concurrent.Callable

interface DataProcessor extends Callable<String> {
    void process(Map<String, Object> data)
}
