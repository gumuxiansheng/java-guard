package com.example;

import java.util.List;
import java.util.Map;

public class UserService {

    private String name;
    private int age;

    public User findById(Long id) {
        System.out.println("Finding user: " + id);
        return null;
    }

    public void process(List<String> items) {
        for (String item : items) {
            if (item != null) {
                System.out.println(item);
            }
        }
    }

    private static class InnerClass {
        public void hello() {
            System.out.println("hello");
        }
    }
}
