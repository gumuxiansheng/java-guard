package com.example;

import java.util.*;
import java.util.List;

public class badClass {
    public void doStuff() {
        System.out.println("hello");
        
        try {
            throw new RuntimeException("oops");
        } catch (Exception e) {
            // empty catch
        }
    }
    
    public void GoodMethod() {
        // method name starts with uppercase
    }
    
    public static final String myConstant = "value";
}
