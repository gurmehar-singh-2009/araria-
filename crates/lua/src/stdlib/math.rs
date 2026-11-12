//! The `math` library, with Luau's `clamp`, `sign`, `round`, `lerp` and
//! `noise` additions.

use std::cell::Cell;
use std::rc::Rc;

use super::{arg, arg_error, int_arg, lib, num_arg, opt_num, register};
use crate::error::{Error, Result};
use crate::value::{Ret, Value, float_to_int_exact};
use crate::vm::Lua;

fn num(v: f64) -> Result<Ret> {
    Ok(Ret::One(Value::Num(v)))
}

/// Floats that hold an integer become integers, as `math.floor` does.
fn int_or_float(v: f64) -> Result<Ret> {
    Ok(Ret::One(match float_to_int_exact(v) {
        Some(i) => Value::Int(i),
        None => Value::Num(v),
    }))
}

fn extreme(args: &[Value], name: &str, pick_greater: bool) -> Result<Ret> {
    if args.is_empty() {
        return Err(arg_error(0, name, "number", &Value::Nil));
    }
    let mut best = args[0].clone();
    num_arg(args, 0, name)?;
    for i in 1..args.len() {
        let candidate = num_arg(args, i, name)?;
        let current = best.to_number().unwrap_or(f64::NAN);
        if (pick_greater && candidate > current) || (!pick_greater && candidate < current) {
            best = args[i].clone();
        }
    }
    Ok(Ret::One(best))
}

/// xoshiro256** — small, fast and good enough for games.
struct Rng(Cell<[u64; 4]>);

impl Rng {
    fn seed(&self, seed: u64) {
        // SplitMix64 expands the seed into the full state.
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        self.0.set([next(), next(), next(), next()]);
    }

    fn next(&self) -> u64 {
        let mut s = self.0.get();
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        self.0.set(s);
        result
    }

    fn float(&self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const PERMUTATION: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99,
    37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32,
    57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27,
    166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244,
    102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130,
    116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147,
    118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
    119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108,
    110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12,
    191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84,
    204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

/// Classic 3D Perlin noise in roughly `-1..=1`.
pub fn perlin(x: f64, y: f64, z: f64) -> f64 {
    let p = |i: usize| PERMUTATION[i & 255] as usize;
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
    let grad = |hash: usize, x: f64, y: f64, z: f64| {
        let h = hash & 15;
        let u = if h < 8 { x } else { y };
        let v = if h < 4 {
            y
        } else if h == 12 || h == 14 {
            x
        } else {
            z
        };
        (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
    };
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (xf as i64 as usize & 255, yf as i64 as usize & 255, zf as i64 as usize & 255);
    let (x, y, z) = (x - xf, y - yf, z - zf);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let a = p(xi) + yi;
    let (aa, ab) = (p(a) + zi, p(a + 1) + zi);
    let b = p(xi + 1) + yi;
    let (ba, bb) = (p(b) + zi, p(b + 1) + zi);
    lerp(
        w,
        lerp(
            v,
            lerp(u, grad(p(aa), x, y, z), grad(p(ba), x - 1.0, y, z)),
            lerp(u, grad(p(ab), x, y - 1.0, z), grad(p(bb), x - 1.0, y - 1.0, z)),
        ),
        lerp(
            v,
            lerp(u, grad(p(aa + 1), x, y, z - 1.0), grad(p(ba + 1), x - 1.0, y, z - 1.0)),
            lerp(u, grad(p(ab + 1), x, y - 1.0, z - 1.0), grad(p(bb + 1), x - 1.0, y - 1.0, z - 1.0)),
        ),
    )
}

pub(super) fn open(lua: &mut Lua) {
    let rng = Rc::new(Rng(Cell::new([0; 4])));
    rng.seed(0x5EED_A2A2_1A00_0001);
    let (rng_random, rng_seed) = (rng.clone(), rng);

    let math = register(
        lua,
        "math",
        lib![
            "abs" => |_, args| Ok(Ret::One(match arg(args, 0) {
                Value::Int(i) => Value::Int(i.wrapping_abs()),
                _ => Value::Num(num_arg(args, 0, "abs")?.abs()),
            })),
            "floor" => |_, args| match arg(args, 0) {
                Value::Int(i) => Ok(Ret::One(Value::Int(*i))),
                _ => int_or_float(num_arg(args, 0, "floor")?.floor()),
            },
            "ceil" => |_, args| match arg(args, 0) {
                Value::Int(i) => Ok(Ret::One(Value::Int(*i))),
                _ => int_or_float(num_arg(args, 0, "ceil")?.ceil()),
            },
            "round" => |_, args| int_or_float(num_arg(args, 0, "round")?.round()),
            "sqrt" => |_, args| num(num_arg(args, 0, "sqrt")?.sqrt()),
            "sin" => |_, args| num(num_arg(args, 0, "sin")?.sin()),
            "cos" => |_, args| num(num_arg(args, 0, "cos")?.cos()),
            "tan" => |_, args| num(num_arg(args, 0, "tan")?.tan()),
            "asin" => |_, args| num(num_arg(args, 0, "asin")?.asin()),
            "acos" => |_, args| num(num_arg(args, 0, "acos")?.acos()),
            "atan" => |_, args| num(num_arg(args, 0, "atan")?.atan2(opt_num(args, 1, "atan", 1.0)?)),
            "atan2" => |_, args| num(num_arg(args, 0, "atan2")?.atan2(num_arg(args, 1, "atan2")?)),
            "exp" => |_, args| num(num_arg(args, 0, "exp")?.exp()),
            "log" => |_, args| {
                let x = num_arg(args, 0, "log")?;
                num(if arg(args, 1).is_nil() {
                    x.ln()
                } else {
                    let base = num_arg(args, 1, "log")?;
                    if base == 2.0 {
                        x.log2()
                    } else if base == 10.0 {
                        x.log10()
                    } else {
                        x.ln() / base.ln()
                    }
                })
            },
            "pow" => |_, args| num(num_arg(args, 0, "pow")?.powf(num_arg(args, 1, "pow")?)),
            "fmod" => |_, args| {
                let (x, y) = (num_arg(args, 0, "fmod")?, num_arg(args, 1, "fmod")?);
                if let (Value::Int(a), Value::Int(b)) = (arg(args, 0), arg(args, 1)) {
                    if *b == 0 {
                        return Err(Error::runtime("invalid argument #2 to 'fmod' (zero)"));
                    }
                    return Ok(Ret::One(Value::Int(a.wrapping_rem(*b))));
                }
                num(x % y)
            },
            "modf" => |_, args| {
                let x = num_arg(args, 0, "modf")?;
                let whole = if x.is_infinite() { x } else { x.trunc() };
                let fraction = if x.is_infinite() { 0.0 } else { x - whole };
                Ok(Ret::Values(vec![Value::Num(whole), Value::Num(fraction)]))
            },
            "deg" => |_, args| num(num_arg(args, 0, "deg")?.to_degrees()),
            "rad" => |_, args| num(num_arg(args, 0, "rad")?.to_radians()),
            "max" => |_, args| extreme(args, "max", true),
            "min" => |_, args| extreme(args, "min", false),
            "clamp" => |_, args| {
                let (lo, hi) = (num_arg(args, 1, "clamp")?, num_arg(args, 2, "clamp")?);
                if lo > hi {
                    return Err(Error::runtime("invalid argument #3 to 'clamp' (max must be greater than or equal to min)"));
                }
                let x = num_arg(args, 0, "clamp")?;
                Ok(Ret::One(if x < lo {
                    args[1].clone()
                } else if x > hi {
                    args[2].clone()
                } else {
                    args[0].clone()
                }))
            },
            "sign" => |_, args| {
                let x = num_arg(args, 0, "sign")?;
                Ok(Ret::One(Value::Int(if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 })))
            },
            "lerp" => |_, args| {
                let (a, b, t) = (num_arg(args, 0, "lerp")?, num_arg(args, 1, "lerp")?, num_arg(args, 2, "lerp")?);
                num(a + (b - a) * t)
            },
            "noise" => |_, args| {
                num(perlin(
                    num_arg(args, 0, "noise")?,
                    opt_num(args, 1, "noise", 0.0)?,
                    opt_num(args, 2, "noise", 0.0)?,
                ))
            },
            "tointeger" => |_, args| Ok(Ret::One(match arg(args, 0) {
                Value::Int(i) => Value::Int(*i),
                Value::Num(n) => float_to_int_exact(*n).map_or(Value::Nil, Value::Int),
                _ => Value::Nil,
            })),
            "type" => |_, args| Ok(Ret::One(match arg(args, 0) {
                Value::Int(_) => Value::str("integer"),
                Value::Num(_) => Value::str("float"),
                Value::Nil if args.is_empty() => return Err(arg_error(0, "type", "value", &Value::Nil)),
                _ => Value::Nil,
            })),
            "ult" => |_, args| Ok(Ret::One(Value::Bool(
                (int_arg(args, 0, "ult")? as u64) < (int_arg(args, 1, "ult")? as u64),
            ))),
            "random" => move |_, args| {
                match args.len() {
                    0 => num(rng_random.float()),
                    1 | 2 => {
                        let (low, high) = if args.len() == 1 {
                            (1, int_arg(args, 0, "random")?)
                        } else {
                            (int_arg(args, 0, "random")?, int_arg(args, 1, "random")?)
                        };
                        if low > high {
                            return Err(Error::runtime("invalid argument to 'random' (interval is empty)"));
                        }
                        let span = (high as i128 - low as i128 + 1) as u128;
                        let offset = (u128::from(rng_random.next()) % span) as i128;
                        Ok(Ret::One(Value::Int((low as i128 + offset) as i64)))
                    }
                    _ => Err(Error::runtime("wrong number of arguments to 'random'")),
                }
            },
            "randomseed" => move |_, args| {
                let seed = match arg(args, 0) {
                    Value::Int(i) => *i as u64,
                    Value::Num(n) => n.to_bits(),
                    _ => 0,
                };
                rng_seed.seed(seed);
                Ok(Ret::None)
            },
        ],
    );
    math.set_str("pi", std::f64::consts::PI);
    math.set_str("huge", f64::INFINITY);
    math.set_str("maxinteger", i64::MAX);
    math.set_str("mininteger", i64::MIN);
}
