package com.example;

public class LoopCases {
    // 正常 for 循环：带 i++ 更新表达式，应当【不】触发 J009
    public void normalForLoop() {
        for (int i = 0; i < 10; i++) {
            System.out.println(i);
        }
    }

    // 死循环：for(;;)，应当触发 J009（确定死循环）
    public void deadForLoop() {
        for (;;) {
            System.out.println("dead");
        }
    }

    // 死循环：while(true)，应当触发 J009（确定死循环）
    public void deadWhileTrue() {
        while (true) {
            System.out.println("dead2");
        }
    }

    // 伪进展：for 无更新表达式且计数变量在循环体内不变，应当触发 J009（#1）
    public void forWithoutUpdate() {
        for (int i = 0; i < 10; ) {
            System.out.println("stuck");
        }
    }
}
