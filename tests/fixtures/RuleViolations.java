package com.example;

import java.util.*;
import java.util.List;
import org.apache.log4j.Logger;

public class badClass {
    public void doStuff() {
        System.out.println("hello");
        
        try {
            throw new RuntimeException("oops");
        } catch (Exception e) {
            // empty catch
        }
    }

    public void silentRethrow() {
        try {
            doStuff();
        } catch (Exception e) {
            // rethrow without slf4j logging
            throw new IllegalStateException(e);
        }
    }
    
    public void GoodMethod() {
        // method name starts with uppercase
    }
    
    public static final String myConstant = "value";
}
