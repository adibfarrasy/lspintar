package com.example;

public interface Notifier {
    void notify(String message);
    void notify(String message, int priority);
}
