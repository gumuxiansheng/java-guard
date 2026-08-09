package com.example.demo;

import org.springframework.web.bind.annotation.*;
import java.util.Map;
import java.util.HashMap;

@RestController
@RequestMapping("/api")
public class UserController {

    // 违规：@RequestBody Map
    @PostMapping("/login")
    public Object login(@RequestBody Map<String, Object> params) {
        return null;
    }

    // 违规：@RequestParam Map
    @GetMapping("/list")
    public Object list(@RequestParam Map<String, String> params) {
        return null;
    }

    // 违规：裸 Map 参数（无注解）
    @PutMapping("/update")
    public void update(Map data) {
    }

    // 违规：HashMap 实现类
    @DeleteMapping("/delete")
    public void delete(HashMap<String, Object> data) {
    }

    // 合规：明确的 DTO
    @PostMapping("/create")
    public Object create(@RequestBody UserCreateReq req) {
        return null;
    }

    // 合规：基础类型参数
    @GetMapping("/get")
    public Object get(@RequestParam String id) {
        return null;
    }
}

// 非 controller 类：不应被 J013 命中
@Component
public class SomeService {
    public void doStuff(Map<String, Object> m) { }
}

// 是 controller，但方法无 @Mapping 注解（不是 HTTP 接口）：不应命中
@RestController
public class NoMappingController {
    public void helper(Map<String, Object> m) { }
}

class UserCreateReq {
    public String name;
}
