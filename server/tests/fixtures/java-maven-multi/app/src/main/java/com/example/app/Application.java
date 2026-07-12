package com.example.app;

import com.example.core.DataProcessor;
import com.example.core.Derived;

public class Application {
    public static void main(String[] args) {
        System.out.println(new DataProcessor().process(" hello "));

        Derived derived = new Derived();
        System.out.println(derived.value());
    }
}
