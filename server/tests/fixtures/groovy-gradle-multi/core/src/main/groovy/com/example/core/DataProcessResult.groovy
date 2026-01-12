package com.example.core

class DataProcessResult {
    enum Status {
        PENDING, SUCCESS, FAILED
    }

    Status status = PENDING
    String message
}
