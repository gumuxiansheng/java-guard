package com.example;

public class BadCode {
    public void riskyMethod() {
        try {
            System.out.println("doing something risky");
        } catch (Exception e) {
            // empty catch block - swallows exception!
        }
    }

    public void anotherMethod() {
        try {
            throw new RuntimeException("oops");
        } catch (RuntimeException e) {
            // also empty!
        }
    }

    public void goodMethod() {
        try {
            System.out.println("fine");
        } catch (Exception e) {
            e.printStackTrace();
        }
    }
}
