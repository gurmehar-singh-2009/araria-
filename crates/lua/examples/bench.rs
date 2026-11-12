//! Micro-benchmark for the interpreter: `cargo run -p araria-lua --release --example bench`.

use std::time::Instant;

use araria_lua::Lua;

const CASES: &[(&str, &str)] = &[
    ("fib(30)", "local function fib(n) if n < 2 then return n end return fib(n-1) + fib(n-2) end return fib(30)"),
    ("loop 10M", "local s = 0 for i = 1, 10000000 do s = s + i % 7 end return s"),
    (
        "table 1M",
        "local t = {} for i = 1, 1000000 do t[i] = i * 2 end local s = 0 for i, v in ipairs(t) do s = s + v end return s",
    ),
    (
        "objects",
        "local P = {} P.__index = P function P.new(x) return setmetatable({x = x}, P) end function P:get() return self.x end \
         local s = 0 for i = 1, 1000000 do s = s + P.new(i):get() end return s",
    ),
    ("strings", "local n = 0 for i = 1, 200000 do n = n + #(('k' .. i):upper()) end return n"),
];

fn main() {
    for (name, source) in CASES {
        let mut lua = Lua::new();
        let start = Instant::now();
        let result = lua.exec(source, name).expect("benchmark chunk runs");
        println!("{name:10} {:>8.1} ms  -> {:?}", start.elapsed().as_secs_f64() * 1000.0, result[0]);
    }
}
