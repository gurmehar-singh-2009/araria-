//! Language conformance tests: each case is a Lua chunk that asserts its own
//! expectations, so a failure reports the Lua source line.

use araria_lua::{ErrorKind, Lua, Value};

fn check(source: &str) {
    let mut lua = Lua::new();
    if let Err(e) = lua.exec(source, "test") {
        panic!("{}", e.report());
    }
}

fn eval(source: &str) -> Vec<Value> {
    let mut lua = Lua::new();
    lua.exec(source, "test").unwrap_or_else(|e| panic!("{}", e.report()))
}

fn error_of(source: &str) -> String {
    let mut lua = Lua::new();
    lua.exec(source, "test").expect_err("expected an error").message()
}

#[test]
fn arithmetic_and_number_types() {
    check(
        r##"
        assert(1 + 2 == 3)
        assert(7 // 2 == 3 and -7 // 2 == -4)
        assert(7 % 3 == 1 and -7 % 3 == 2 and 7 % -3 == -2)
        assert(7 / 2 == 3.5)
        assert(2 ^ 10 == 1024.0)
        assert(math.type(1) == "integer" and math.type(1.0) == "float")
        assert(math.type(3 / 1) == "float")
        assert(1 == 1.0)
        assert(tostring(10 / 2) == "5.0" and tostring(3) == "3")
        assert(10 // 0.0 == math.huge)
        assert(5.5 % 2 == 1.5)
        assert("10" + 5 == 15)
        assert(0x10 == 16 and 1e2 == 100.0)
        assert(3 & 5 == 1 and 3 | 5 == 7 and 3 ~ 5 == 6 and ~0 == -1)
        assert(1 << 4 == 16 and 256 >> 4 == 16)
        assert(math.maxinteger + 1 == math.mininteger)
        assert(-2 ^ 2 == -4.0)
        assert(2 ^ 3 ^ 2 == 512.0)
        assert(1 .. 2 == "12")
        assert(tostring(1e100) == "1e+100")
        assert(tostring(0.1) == "0.1")
        assert(tostring(-0.5) == "-0.5")
        assert(#"hello" == 5)
        assert(not nil and not false and 0 and "")
        "##,
    );
}

#[test]
fn comparison_and_logic() {
    check(
        r##"
        assert(1 < 2 and 2 <= 2 and 3 > 2 and 3 >= 3)
        assert("a" < "b" and "abc" < "abd" and "Z" < "a")
        assert(1 < 1.5 and 2.5 > 2)
        assert((nil or 5) == 5 and (false and 5) == false)
        assert((1 and 2) == 2 and (nil and 2) == nil)
        local a = nil
        local b = a and a.x or "default"
        assert(b == "default")
        assert(1 ~= 2 and "1" ~= 1)
        "##,
    );
    assert!(error_of("return 1 < 'x'").contains("attempt to compare number with string"));
    assert!(error_of("return {} < {}").contains("attempt to compare two table values"));
}

#[test]
fn locals_closures_and_upvalues() {
    check(
        r##"
        local function counter()
            local n = 0
            return function() n = n + 1 return n end
        end
        local c1, c2 = counter(), counter()
        assert(c1() == 1 and c1() == 2 and c2() == 1)

        -- each loop iteration captures a fresh variable
        local fns = {}
        for i = 1, 3 do fns[i] = function() return i end end
        assert(fns[1]() == 1 and fns[2]() == 2 and fns[3]() == 3)

        local fs = {}
        for _, v in ipairs({"a", "b"}) do fs[#fs + 1] = function() return v end end
        assert(fs[1]() == "a" and fs[2]() == "b")

        -- shared upvalue between two closures
        local function pair()
            local v = 0
            return function() v = v + 1 end, function() return v end
        end
        local inc, get = pair()
        inc() inc()
        assert(get() == 2)

        -- recursion through a local function
        local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end
        assert(fib(20) == 6765)

        -- nested upvalues through an intermediate function
        local function outer()
            local x = 10
            return function()
                return function() x = x + 1 return x end
            end
        end
        assert(outer()()() == 11)

        -- parameters can be captured
        local function adder(a) return function(b) return a + b end end
        assert(adder(3)(4) == 7)

        -- shadowing
        local x = 1
        do local x = 2 assert(x == 2) end
        assert(x == 1)
        local x = x + 1
        assert(x == 2)
        "##,
    );
}

#[test]
fn multiple_assignment_and_varargs() {
    check(
        r##"
        local a, b, c = 1, 2
        assert(a == 1 and b == 2 and c == nil)
        a, b = b, a
        assert(a == 2 and b == 1)
        local function three() return 1, 2, 3 end
        local x, y, z, w = three()
        assert(x == 1 and y == 2 and z == 3 and w == nil)
        local p, q = three(), 10
        assert(p == 1 and q == 10)
        local t = {three(), three()}
        assert(#t == 4)
        local u = {(three())}
        assert(#u == 1)
        local function count(...) return select("#", ...) end
        assert(count() == 0 and count(nil, nil) == 2 and count(three()) == 3)
        local function pass(...) return ... end
        assert(select("#", pass(1, nil, 3)) == 3)
        local function sum(...)
            local s = 0
            for _, v in ipairs({...}) do s = s + v end
            return s
        end
        assert(sum(1, 2, 3, 4) == 10)
        local function first(...) local a = ... return a end
        assert(first(7, 8) == 7)
        assert(select(-1, 1, 2, 3) == 3)
        local tt = {}
        tt.a, tt.b = 1, 2
        assert(tt.a == 1 and tt.b == 2)
        local i = 1
        local arr = {}
        i, arr[i] = i + 1, 20
        assert(i == 2 and arr[1] == 20)
        "##,
    );
}

#[test]
fn control_flow() {
    check(
        r##"
        local s = 0
        for i = 1, 10 do s = s + i end
        assert(s == 55)
        s = 0
        for i = 10, 1, -2 do s = s + i end
        assert(s == 30)
        s = 0
        for i = 1, 0 do s = s + 1 end
        assert(s == 0)
        local n = 0
        for x = 0, 1, 0.25 do n = n + 1 end
        assert(n == 5)

        local i = 0
        while true do i = i + 1 if i >= 5 then break end end
        assert(i == 5)

        i = 0
        repeat local done = i >= 3 i = i + 1 until done
        assert(i == 4)

        -- continue
        s = 0
        for k = 1, 10 do
            if k % 2 == 0 then continue end
            s = s + k
        end
        assert(s == 25)
        i, s = 0, 0
        while i < 5 do
            i = i + 1
            local skip = i == 3
            if skip then continue end
            s = s + i
        end
        assert(s == 12)

        -- goto continue idiom, skipping a local declaration
        s = 0
        for k = 1, 5 do
            local half = k / 2
            if k == 2 then goto continue end
            local extra = 1
            s = s + k + extra - 1
            ::continue::
        end
        assert(s == 13)

        -- backward goto
        local c = 0
        ::top::
        c = c + 1
        if c < 3 then goto top end
        assert(c == 3)

        -- nested break
        local found
        for a = 1, 3 do
            for b = 1, 3 do
                if a * b == 6 then found = {a, b} break end
            end
            if found then break end
        end
        assert(found[1] == 2 and found[2] == 3)

        if false then error("no") elseif nil then error("no") else s = "else" end
        assert(s == "else")

        -- continue as an ordinary identifier still works
        local continue = 5
        continue = continue + 1
        assert(continue == 6)
        "##,
    );
}

#[test]
fn tables() {
    check(
        r##"
        local t = {10, 20, 30, x = 1, ["y z"] = 2, [100] = "h"}
        assert(#t == 3 and t.x == 1 and t["y z"] == 2 and t[100] == "h")
        t[#t + 1] = 40
        assert(#t == 4)
        t[2.0] = "two"
        assert(t[2] == "two")
        local keys = 0
        for k, v in pairs(t) do keys = keys + 1 end
        assert(keys == 7)
        -- removing during traversal is allowed
        for k in pairs(t) do t[k] = nil end
        assert(next(t) == nil)

        -- sparse tables migrate into the array part
        local s = {}
        s[3] = "c" s[2] = "b" s[1] = "a"
        assert(#s == 3)

        local nested = {a = {b = {c = 42}}}
        assert(nested.a.b.c == 42)
        nested.a.b.c += 1
        assert(nested.a.b.c == 43)
        local str = "x"
        str ..= "y"
        assert(str == "xy")

        assert(rawlen({1, 2}) == 2 and rawequal(t, t) and rawget({5}, 1) == 5)
        local ok = pcall(function() local z = nil; z.field = 1 end)
        assert(not ok)
        local big = {}
        for i = 1, 200 do big[i] = i end
        assert(#big == 200 and big[200] == 200)
        local long = {1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,
            31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60}
        assert(#long == 60 and long[49] == 49 and long[60] == 60)
        "##,
    );
    assert!(error_of("local t = {} t[nil] = 1").contains("table index is nil"));
}

#[test]
fn metatables() {
    check(
        r##"
        local Vec = {}
        Vec.__index = Vec
        function Vec.new(x, y) return setmetatable({x = x, y = y}, Vec) end
        function Vec:len() return math.sqrt(self.x ^ 2 + self.y ^ 2) end
        Vec.__add = function(a, b) return Vec.new(a.x + b.x, a.y + b.y) end
        Vec.__eq = function(a, b) return a.x == b.x and a.y == b.y end
        Vec.__lt = function(a, b) return a:len() < b:len() end
        Vec.__le = function(a, b) return a:len() <= b:len() end
        Vec.__tostring = function(v) return "(" .. v.x .. ", " .. v.y .. ")" end
        Vec.__len = function() return 2 end
        Vec.__call = function(self, k) return self[k] end
        Vec.__concat = function(a, b) return tostring(a) .. "|" .. tostring(b) end
        Vec.__unm = function(a) return Vec.new(-a.x, -a.y) end
        local a, b = Vec.new(3, 4), Vec.new(1, 2)
        assert(a:len() == 5)
        assert((a + b).x == 4)
        assert(a == Vec.new(3, 4) and a ~= b)
        assert(b < a and b <= a and a > b)
        assert(tostring(a) == "(3, 4)")
        assert(#a == 2 and a("x") == 3)
        assert(a .. b == "(3, 4)|(1, 2)")
        assert((-a).x == -3)

        -- __index / __newindex functions
        local log = {}
        local proxy = setmetatable({}, {
            __index = function(_, k) return k .. "!" end,
            __newindex = function(t, k, v) rawset(t, k, v * 2) log[#log + 1] = k end,
        })
        assert(proxy.hello == "hello!")
        proxy.n = 21
        assert(proxy.n == 42 and log[1] == "n")

        -- inheritance chain
        local Base = {} Base.__index = Base
        function Base:greet() return "hi " .. self.name end
        local Derived = setmetatable({}, {__index = Base}) Derived.__index = Derived
        local obj = setmetatable({name = "bob"}, Derived)
        assert(obj:greet() == "hi bob")

        assert(getmetatable(a) == Vec)
        assert(getmetatable("").__index == string)
        local locked = setmetatable({}, {__metatable = "locked"})
        assert(getmetatable(locked) == "locked")
        assert(not pcall(setmetatable, locked, {}))
        "##,
    );
}

#[test]
fn strings_and_patterns() {
    check(
        r##"
        assert(("hello"):upper() == "HELLO" and string.lower("ABC") == "abc")
        assert(("hello"):sub(2, 4) == "ell" and ("hello"):sub(-3) == "llo" and ("hello"):sub(10) == "")
        assert(("ab"):rep(3, "-") == "ab-ab-ab" and ("abc"):reverse() == "cba")
        assert(("A"):byte() == 65 and string.char(72, 105) == "Hi")
        assert(#("\u{48}\x41\65\n") == 4)
        assert([[long
string]] == "long\nstring")

        assert(string.find("hello world", "wor") == 7)
        local s, e = string.find("hello world", "o w")
        assert(s == 5 and e == 7)
        assert(string.find("a.b", ".", 1, true) == 2)
        assert(string.find("abc", "x") == nil)
        assert(string.match("key = value", "(%w+)%s*=%s*(%w+)") == "key")
        local k, v = string.match("key = value", "(%w+)%s*=%s*(%w+)")
        assert(k == "key" and v == "value")
        assert(string.match("2024-01-15", "(%d+)-(%d+)-(%d+)") == "2024")
        assert(string.match("  trim  ", "^%s*(.-)%s*$") == "trim")
        assert(string.match("hello", "l+") == "ll")
        assert(string.match("[tag]", "%[(.-)%]") == "tag")
        assert(string.match("f(a(b)c)", "%b()") == "(a(b)c)")
        assert(string.match("THE (quick) fox", "%f[%a]%a+%f[%A]") == "THE")
        assert(string.match("abc", "()b()") == 2)
        assert(string.match("x=1", "[%w_]+") == "x")
        assert(string.match("hello", ".-(l+)(.*)") == "ll")
        assert(string.match("aaa", "a-b") == nil)
        assert(string.match("abcabc", "(abc)%1") == "abc")

        local words = {}
        for w in string.gmatch("one two  three", "%a+") do words[#words + 1] = w end
        assert(#words == 3 and words[3] == "three")
        local pairs_found = {}
        for key, val in string.gmatch("a=1, b=2", "(%w+)=(%w+)") do pairs_found[key] = val end
        assert(pairs_found.a == "1" and pairs_found.b == "2")

        assert(string.gsub("hello world", "o", "0") == "hell0 w0rld")
        local r, n = string.gsub("hello", "l", "L", 1)
        assert(r == "heLlo" and n == 1)
        assert(string.gsub("abc", "%w", "%0%0") == "aabbcc")
        assert(string.gsub("hello world", "(%w+) (%w+)", "%2 %1") == "world hello")
        assert(string.gsub("$name is $age", "%$(%w+)", {name = "Ann", age = 30}) == "Ann is 30")
        assert(string.gsub("abc", "%w", function(c) return c:upper() .. "." end) == "A.B.C.")
        assert(string.gsub("abc", "", "-") == "-a-b-c-")
        assert(string.gsub("  x  ", "^%s+", "") == "x  ")

        assert(string.format("%d %s %5.2f", 42, "hi", 3.14159) == "42 hi  3.14")
        assert(string.format("%05d|%-5d|%+d", 42, 42, 42) == "00042|42   |+42")
        assert(string.format("%x %X %o", 255, 255, 8) == "ff FF 10")
        assert(string.format("%q", 'a "b"') == '"a \\"b\\""')
        assert(string.format("%10s|%-10s|", "hi", "hi") == "        hi|hi        |")
        assert(string.format("%.3s", "abcdef") == "abc")
        assert(string.format("%g %g %g", 100, 0.5, 1e20) == "100 0.5 1e+20")
        assert(string.format("%e", 12345.678) == "1.234568e+04")
        assert(string.format("%5.1f%%", 99.55) == " 99.5%" or string.format("%5.1f%%", 99.55) == " 99.6%")
        assert(string.format("%c%c", 72, 105) == "Hi")
        assert(string.format("%s %s", nil, true) == "nil true")
        assert(string.format("%i", 3.0) == "3")

        local parts = string.split("a,b,,c", ",")
        assert(#parts == 4 and parts[3] == "" and parts[4] == "c")
        assert(tostring(nil) == "nil" and tostring(true) == "true")
        assert(tonumber("0x1F") == 31 and tonumber("  12  ") == 12 and tonumber("1e1") == 10.0)
        assert(tonumber("abc") == nil and tonumber("ff", 16) == 255 and tonumber("z", 36) == 35)
        assert(tonumber("") == nil and tonumber("1 2") == nil)
        assert(utf8.char(228, 8364) == "ä€" and utf8.len("ä€") == 2)
        "##,
    );
    assert!(error_of("string.rep('x', 1e10)").contains("too large"));
    assert!(error_of("string.find('a', '[a')").contains("malformed pattern"));
}

#[test]
fn table_library() {
    check(
        r##"
        local t = {3, 1, 2}
        table.insert(t, 4)
        table.insert(t, 1, 0)
        assert(table.concat(t, ",") == "0,3,1,2,4")
        assert(table.remove(t) == 4 and table.remove(t, 1) == 0)
        table.sort(t)
        assert(table.concat(t, " ") == "1 2 3")
        table.sort(t, function(a, b) return a > b end)
        assert(table.concat(t) == "321")
        local words = {"pear", "apple", "fig"}
        table.sort(words)
        assert(words[1] == "apple" and words[3] == "pear")
        local records = {{n = "b", v = 2}, {n = "a", v = 2}, {n = "c", v = 1}}
        table.sort(records, function(x, y) return x.v < y.v end)
        assert(records[1].n == "c" and records[2].n == "b" and records[3].n == "a")
        -- an inconsistent comparator must not crash the host
        local junk = {}
        for i = 1, 50 do junk[i] = (i * 7919) % 13 end
        table.sort(junk, function() return true end)
        assert(#junk == 50)

        assert(select("#", table.unpack({1, 2, 3})) == 3)
        assert(table.pack(1, nil, 3).n == 3)
        assert(table.find({"a", "b", "c"}, "b") == 2 and table.find({1}, 9) == nil)
        local copy = table.clone({1, 2, x = 3})
        assert(copy[2] == 2 and copy.x == 3)
        table.clear(copy)
        assert(next(copy) == nil)
        assert(#table.create(5, 0) == 5)
        local moved = table.move({1, 2, 3}, 1, 3, 2, {})
        assert(moved[2] == 1 and moved[4] == 3)
        local big = {}
        for i = 1, 1000 do big[i] = (i * 7919) % 1000 end
        table.sort(big)
        for i = 2, 1000 do assert(big[i - 1] <= big[i]) end
        "##,
    );
    assert!(error_of("table.sort({1, 'x'})").contains("attempt to compare"));
}

#[test]
fn math_library() {
    check(
        r##"
        assert(math.floor(3.7) == 3 and math.ceil(3.2) == 4 and math.floor(-3.5) == -4)
        assert(math.type(math.floor(3.7)) == "integer")
        assert(math.max(1, 5, 3) == 5 and math.min(4, 2, 8) == 2)
        assert(math.abs(-4) == 4 and math.sqrt(16) == 4.0)
        assert(math.clamp(15, 0, 10) == 10 and math.clamp(-1, 0, 10) == 0 and math.clamp(5, 0, 10) == 5)
        assert(math.sign(-3) == -1 and math.sign(0) == 0 and math.round(2.5) == 3)
        assert(math.lerp(0, 10, 0.5) == 5.0)
        assert(math.abs(math.sin(math.pi)) < 1e-12 and math.cos(0) == 1.0)
        assert(math.huge > math.maxinteger and -math.huge < math.mininteger)
        assert(math.fmod(7, 3) == 1 and math.tointeger(3.0) == 3 and math.tointeger(3.5) == nil)
        math.randomseed(42)
        local a = {math.random(), math.random(10), math.random(5, 6)}
        math.randomseed(42)
        local b = {math.random(), math.random(10), math.random(5, 6)}
        assert(a[1] == b[1] and a[2] == b[2] and a[3] == b[3])
        for _ = 1, 200 do
            local r = math.random(3, 7)
            assert(r >= 3 and r <= 7 and math.type(r) == "integer")
            local f = math.random()
            assert(f >= 0 and f < 1)
        end
        local n = math.noise(1.5, 2.5, 3.5)
        assert(n >= -1 and n <= 1 and math.noise(1.5, 2.5, 3.5) == n and math.noise(0, 0, 0) == 0)
        "##,
    );
}

#[test]
fn errors_and_protected_calls() {
    check(
        r##"
        local ok, err = pcall(error, "boom")
        assert(not ok and err == "boom")
        ok, err = pcall(function() error("with position") end)
        assert(not ok and err == "test:4: with position", err)
        ok, err = pcall(function() error("no position", 0) end)
        assert(err == "no position")
        ok, err = pcall(function() error({code = 7}) end)
        assert(type(err) == "table" and err.code == 7)
        ok, err = pcall(function() local t = nil; return t.x end)
        assert(not ok and err:find("attempt to index nil with 'x'"), err)
        ok, err = pcall(function() undefined_function() end)
        assert(err:find("attempt to call a nil value %(global 'undefined_function'%)"), err)
        ok, err = pcall(function() local t = {} t:nope() end)
        assert(err:find("method 'nope'"), err)
        ok, err = pcall(function() return 1 + {} end)
        assert(err:find("attempt to perform arithmetic %(add%) on a table value"), err)
        ok, err = pcall(function() return "a" .. {} end)
        assert(err:find("attempt to concatenate a table value"), err)
        ok, err = pcall(function() return #5 end)
        assert(err:find("attempt to get length of a number value"), err)

        -- pcall returns all results
        local a, b, c = pcall(function() return 1, 2 end)
        assert(a == true and b == 1 and c == 2)
        -- nested pcall
        ok, err = pcall(function()
            local inner_ok, inner_err = pcall(error, "inner")
            assert(not inner_ok and inner_err == "inner")
            error("outer", 0)
        end)
        assert(not ok and err == "outer")
        -- xpcall with handler
        local handled
        ok, err = xpcall(function() error("x", 0) end, function(m) handled = m return "handled:" .. m end)
        assert(not ok and err == "handled:x" and handled == "x")
        ok, err = xpcall(function() error("trace me") end, debug.traceback)
        assert(not ok and err:find("stack traceback"), err)
        assert(select("#", assert(1, 2, 3)) == 3)
        ok, err = pcall(assert, false, "custom")
        assert(err == "custom")
        ok, err = pcall(assert, nil)
        assert(err:find("assertion failed"))
        -- error with level 2 blames the caller
        local function check_arg(x) if not x then error("bad arg", 2) end end
        ok, err = pcall(function()
            check_arg(nil)
        end)
        assert(err:find("^test:%d+: bad arg"), err)
        -- runtime error deep in recursion
        local function deep(n) if n == 0 then error("bottom", 0) end return deep(n - 1) end
        ok, err = pcall(deep, 50)
        assert(err == "bottom")
        -- stack overflow is catchable
        local function forever() return 1 + forever() end
        ok, err = pcall(forever)
        assert(not ok and err:find("stack overflow"), err)
        "##,
    );
}

#[test]
fn error_positions_and_traceback() {
    let mut lua = Lua::new();
    let err = lua
        .exec("local function inner()\n  local x = nil\n  return x.field\nend\nlocal function outer()\n  return inner()\nend\nouter()", "game")
        .unwrap_err();
    assert_eq!(err.message(), "game:3: attempt to index nil with 'field'");
    assert_eq!(err.line, Some(3));
    assert_eq!(err.chunk.as_deref(), Some("game"));
    let lines: Vec<u32> = err.traceback.iter().map(|t| t.line).collect();
    assert_eq!(lines, vec![3, 6, 8]);
    assert!(err.traceback[0].function.contains("inner"));
    assert!(err.report().contains("stack traceback"));

    let syntax = lua.load("local x = \nlocal y = = 2", "broken", None).unwrap_err();
    assert_eq!(syntax.kind, ErrorKind::Syntax);
    assert_eq!(syntax.line, Some(2));
    assert!(syntax.message().starts_with("broken:2:"), "{}", syntax.message());

    for (source, expect) in [
        ("x = = 1", "unexpected symbol"),
        ("if x then", "'end' expected"),
        ("for i = 1 do end", "',' expected"),
        ("local t = {1, 2", "'}' expected"),
        ("break", "break outside a loop"),
        ("goto nowhere", "no visible label"),
        ("x = 'unfinished", "unfinished string"),
        ("return return", "unexpected symbol"),
        ("f(", "unexpected symbol"),
        ("local 5 = 1", "'<name>' expected"),
        ("x = 1 2", "unexpected symbol"),
    ] {
        let e = lua.load(source, "s", None).expect_err(source);
        assert!(e.message().contains(expect), "{source}: {}", e.message());
    }
}

#[test]
fn coroutines() {
    check(
        r##"
        local co = coroutine.create(function(a, b)
            local c = coroutine.yield(a + b)
            local d, e = coroutine.yield(c * 2)
            return d + e
        end)
        assert(coroutine.status(co) == "suspended")
        local ok, v = coroutine.resume(co, 1, 2)
        assert(ok and v == 3)
        ok, v = coroutine.resume(co, 10)
        assert(ok and v == 20)
        ok, v = coroutine.resume(co, 3, 4)
        assert(ok and v == 7 and coroutine.status(co) == "dead")
        ok, v = coroutine.resume(co)
        assert(not ok and v:find("dead"))

        -- generators with wrap
        local function range(n)
            return coroutine.wrap(function() for i = 1, n do coroutine.yield(i) end end)
        end
        local sum = 0
        for i in range(5) do sum = sum + i end
        assert(sum == 15)

        -- yield across pcall
        co = coroutine.wrap(function()
            local ok, err = pcall(function()
                coroutine.yield("inside")
                error("after yield", 0)
            end)
            coroutine.yield(ok, err)
            return "done"
        end)
        assert(co() == "inside")
        local a, b = co()
        assert(a == false and b == "after yield")
        assert(co() == "done")

        -- errors inside coroutines
        co = coroutine.create(function() error("oops", 0) end)
        ok, v = coroutine.resume(co)
        assert(not ok and v == "oops" and coroutine.status(co) == "dead")

        -- nested coroutines and status
        local outer
        outer = coroutine.create(function()
            local inner = coroutine.create(function()
                assert(coroutine.status(outer) == "normal")
                coroutine.yield(1)
            end)
            coroutine.resume(inner)
            assert(coroutine.status(inner) == "suspended")
            assert(coroutine.running() == outer)
            coroutine.yield(2)
        end)
        ok, v = coroutine.resume(outer)
        assert(ok and v == 2, v)
        assert(coroutine.isyieldable() == false)
        assert(not pcall(coroutine.yield))
        assert(coroutine.close(outer) and coroutine.status(outer) == "dead")

        -- yield inside loops with many values
        co = coroutine.wrap(function(...)
            local args = {...}
            while true do
                local x, y = coroutine.yield(table.unpack(args))
                args = {x, y, x + y}
            end
        end)
        assert(select("#", co(1, 2, 3)) == 3)
        local p, q, r = co(4, 5)
        assert(p == 4 and q == 5 and r == 9)
        "##,
    );
}

#[test]
fn execution_budget_stops_runaway_scripts() {
    let mut lua = Lua::new();
    lua.set_budget(100_000);
    let err = lua.exec("while true do end", "loop").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Budget);

    // pcall cannot swallow the budget error.
    lua.set_budget(100_000);
    let err = lua.exec("while true do pcall(function() while true do end end) end", "loop").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Budget);

    // Nor can a coroutine.
    lua.set_budget(100_000);
    let err = lua
        .exec("local co = coroutine.wrap(function() while true do end end) pcall(co) while true do end", "loop")
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Budget);

    // Recursion without loops is also bounded.
    lua.set_budget(100_000);
    let err = lua.exec("local function f() return pcall(f) end f() while true do f() end", "loop").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Budget);

    // The state stays usable afterwards.
    lua.set_budget(1_000_000);
    assert_eq!(lua.exec("return 1 + 1", "ok").unwrap()[0], Value::Int(2));
}

#[test]
fn garbage_collector_reclaims_cycles() {
    let mut lua = Lua::new();
    lua.exec(
        r##"
        keep = {}
        keep.self = keep
        for i = 1, 5000 do
            local a, b = {}, {}
            a.other, b.other = b, a
            local function recursive() return recursive end
            a.f = recursive
        end
        "##,
        "gc",
    )
    .unwrap();
    let stats = lua.collect_garbage();
    assert!(stats.tables < 200, "cyclic tables were not collected: {stats:?}");
    assert!(stats.closures < 400, "cyclic closures were not collected: {stats:?}");
    // Reachable cycles survive.
    let r = lua.exec("return keep.self.self == keep, type(string.format)", "gc").unwrap();
    assert_eq!(r[0], Value::Bool(true));

    // Values held only by the host survive collection.
    let held = lua.exec("local t = {secret = 7} t.me = t return t", "gc").unwrap();
    lua.collect_garbage();
    assert_eq!(held[0].as_table().unwrap().get_str("secret"), Value::Int(7));

    // Suspended coroutines keep their locals alive.
    lua.exec(
        r##"
        gen = coroutine.wrap(function()
            local state = {count = 0}
            state.loop = state
            while true do state.count = state.count + 1 coroutine.yield(state.count) end
        end)
        gen()
        "##,
        "gc",
    )
    .unwrap();
    lua.collect_garbage();
    assert_eq!(lua.exec("return gen()", "gc").unwrap()[0], Value::Int(2));
}

#[test]
fn sandbox_has_no_host_access() {
    let globals = eval("return io, os.execute, os.getenv, os.remove, require, dofile, loadfile, package");
    assert!(globals.iter().all(Value::is_nil), "{globals:?}");
    check(
        r##"
        assert(type(os.time()) == "number" and type(os.clock()) == "number")
        assert(os.date("%Y-%m-%d", 0) == "1970-01-01")
        assert(os.date("%H:%M:%S", 3661) == "01:01:01")
        assert(os.time({year = 2000, month = 1, day = 1, hour = 0}) == 946684800)
        local f = load("return 1 + 1")
        assert(f() == 2)
        local bad, msg = load("return +")
        assert(bad == nil and type(msg) == "string")
        local env = {y = 5}
        assert(load("return y", "chunk", "t", env)() == 5)
        "##,
    );
}

#[test]
fn isolated_environments() {
    let mut lua = Lua::new();
    let make_env = |lua: &mut Lua| {
        let env = lua.new_table();
        let meta = lua.new_table();
        meta.set_str("__index", lua.globals());
        env.borrow_mut().set_metatable(Some(meta));
        env
    };
    let (env_a, env_b) = (make_env(&mut lua), make_env(&mut lua));
    let a = lua.load("score = 10 return math.max(score, 1)", "a", Some(env_a.clone())).unwrap();
    let b = lua.load("return score", "b", Some(env_b)).unwrap();
    assert_eq!(lua.call(&a, &[]).unwrap()[0], Value::Int(10));
    assert_eq!(lua.call(&b, &[]).unwrap()[0], Value::Nil);
    assert_eq!(env_a.get_str("score"), Value::Int(10));
    assert!(lua.get_global("score").is_nil());
}

#[test]
fn vectors_are_values() {
    let mut lua = Lua::new();
    lua.set_global(
        "vec",
        Lua::native("vec", |_, args| {
            let n = |i: usize| args.get(i).and_then(Value::to_number).unwrap_or(0.0) as f32;
            Ok(araria_lua::Ret::One(Value::Vector(n(0), n(1), n(2))))
        }),
    );
    let r = lua
        .exec(
            r##"
            local a, b = vec(1, 2, 3), vec(4, 5, 6)
            local c = a + b * 2 - vec(1, 1, 1)
            assert(c == vec(8, 11, 14))
            assert((a * b) == vec(4, 10, 18) and (b / 2) == vec(2, 2.5, 3) and (2 * a) == vec(2, 4, 6))
            assert(-a == vec(-1, -2, -3))
            assert(a.X == 1 and a.y == 2 and a.Z == 3)
            local set = {[vec(1, 2, 3)] = "found"}
            assert(set[a] == "found")
            assert(type(a) == "vector")
            return c
            "##,
            "vec",
        )
        .unwrap_or_else(|e| panic!("{}", e.report()));
    assert_eq!(r[0], Value::Vector(8.0, 11.0, 14.0));
}

#[test]
fn host_interop() {
    let mut lua = Lua::new();
    let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = captured.clone();
    lua.set_print_hook(move |line| sink.borrow_mut().push(line.to_string()));
    lua.exec("print('a', 1, nil, 2.5) print(setmetatable({}, {__tostring = function() return 'custom' end}))", "p")
        .unwrap();
    assert_eq!(*captured.borrow(), vec!["a\t1\tnil\t2.5".to_string(), "custom".to_string()]);

    // Host calls into Lua and gets multiple results back.
    let f = lua.exec("return function(a, b) return a + b, a * b end", "f").unwrap().remove(0);
    let out = lua.call(&f, &[Value::Int(3), Value::Int(4)]).unwrap();
    assert_eq!(out, vec![Value::Int(7), Value::Int(12)]);

    // A yield with no coroutine to yield to is an error, not a hang.
    lua.set_global("host_yield", Lua::native("host_yield", |_, _| Ok(araria_lua::Ret::Yield(vec![]))));
    assert!(lua.exec("host_yield()", "y").unwrap_err().message().contains("yield"));

    // The host can drive coroutines itself (this is how the scheduler works).
    let body = lua.exec("return function(x) local y = host_yield('first') return x + y end", "co").unwrap().remove(0);
    let co = lua.create_thread(body);
    match lua.resume(&co, vec![Value::Int(1)]).unwrap() {
        araria_lua::Resumed::Yield(v) => assert_eq!(v, Vec::<Value>::new()),
        other => panic!("expected yield, got {other:?}"),
    }
    match lua.resume(&co, vec![Value::Int(41)]).unwrap() {
        araria_lua::Resumed::Return(v) => assert_eq!(v, vec![Value::Int(42)]),
        other => panic!("expected return, got {other:?}"),
    }
}
