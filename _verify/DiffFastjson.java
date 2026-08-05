package com.example.demo;

import com.alibaba.fastjson.JSON;

public class DiffFastjson {
    public void oldMethod() {
        int x = 1;
    }

    public void newSimpleNameUsage() {
        Object a = JSON.parseObject("{}");
    }

    public void newFullyQualifiedUsage() {
        Object b = com.alibaba.fastjson.JSONObject.toJSONString("x");
    }
}
