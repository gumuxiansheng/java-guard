package com.example.demo;

import com.alibaba.fastjson.JSON;
import com.fasterxml.jackson.databind.ObjectMapper;

public class Sample {
    public void fullyQualifiedNoImport() {
        Object a = com.alibaba.fastjson.JSON.parseObject("{}");
        Object b = new com.alibaba.fastjson2.JSONObject();
    }

    public void simpleNameWithImport() {
        Object c = JSON.parseObject("{}");
    }

    public void castUsage() {
        Object d = (com.alibaba.fastjson.JSONObject) getObj();
    }

    public void safeJackson() {
        ObjectMapper mapper = new ObjectMapper();
        String s = String.valueOf(123);
    }

    private Object getObj() { return null; }
}
