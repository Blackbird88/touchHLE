/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `stdlib.h`

use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, ConstVoidPtr, GuestUSize, MutPtr, MutVoidPtr, Ptr};
use crate::{Environment, export_c_func2};
use std::collections::HashMap;
use std::io::Write;
use std::str::FromStr;
use crate::libc::posix_io::getcwd;
use crate::libc::string::{strlen, strcpy};
use crate::libc::wchar::{wchar_t, wmemcpy};

pub mod qsort;

#[derive(Default)]
pub struct State {
    rand: u32,
    random: u32,
    arc4random: u32,
    env: HashMap<Vec<u8>, MutPtr<u8>>,
}

// Sizes of zero are implementation-defined. macOS will happily give you back
// an allocation for any of these, so presumably iPhone OS does too.
// (touchHLE's allocator will round up allocations to at least 16 bytes.)

fn malloc(env: &mut Environment, size: GuestUSize) -> MutVoidPtr {
    env.mem.alloc(size)
}

fn calloc(env: &mut Environment, count: GuestUSize, size: GuestUSize) -> MutVoidPtr {
    let total = size.checked_mul(count).unwrap();
    env.mem.alloc(total)
}

fn realloc(env: &mut Environment, ptr: MutVoidPtr, size: GuestUSize) -> MutVoidPtr {
    if ptr.is_null() {
        return malloc(env, size);
    }
    env.mem.realloc(ptr, size)
}

fn free(env: &mut Environment, ptr: MutVoidPtr) {
    if ptr.is_null() {
        // "If ptr is a NULL pointer, no operation is performed."
        return;
    }
    env.mem.free(ptr);
}

fn atexit(
    _env: &mut Environment,
    func: GuestFunction, // void (*func)(void)
) -> i32 {
    // TODO: when this is implemented, make sure it's properly compatible with
    // __cxa_atexit.
    log!("TODO: atexit({:?}) (unimplemented)", func);
    0 // success
}

fn skip_whitespace(env: &mut Environment, s: ConstPtr<u8>) -> ConstPtr<u8> {
    let mut start = s;
    loop {
        let c = env.mem.read(start);
        // Rust's definition of whitespace excludes vertical tab, unlike C's
        if c.is_ascii_whitespace() || c == b'\x0b' {
            start += 1;
        } else {
            break;
        }
    }
    start
}

fn atoi(env: &mut Environment, s: ConstPtr<u8>) -> i32 {
    // atoi() doesn't work with a null-terminated string, instead it stops
    // once it hits something that's not a digit, so we have to do some parsing
    // ourselves.
    let start = skip_whitespace(env, s);
    let mut len = 0;
    let maybe_sign = env.mem.read(start + len);
    if maybe_sign == b'+' || maybe_sign == b'-' || maybe_sign.is_ascii_digit() {
        len += 1;
    }
    while env.mem.read(start + len).is_ascii_digit() {
        len += 1;
    }

    let s = std::str::from_utf8(env.mem.bytes_at(start, len)).unwrap();
    // conveniently, overflow is undefined, so 0 is as valid a result as any
    s.parse().unwrap_or(0)
}

fn atol(env: &mut Environment, s: ConstPtr<u8>) -> i32 {
    atoi(env, s)
}

fn atof(env: &mut Environment, s: ConstPtr<u8>) -> f64 {
    atof_inner(env, s).map_or(0.0, |tuple| tuple.0)
}

fn prng(state: u32) -> u32 {
    // The state must not be zero for this algorithm to work. This also makes
    // the default seed be 1, which matches the C standard.
    let mut state: u32 = state.max(1);
    // https://en.wikipedia.org/wiki/Xorshift#Example_implementation
    // xorshift32 is not a good random number generator, but it is cute one!
    // It's not like anyone expects the C stdlib `rand()` to be good.
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    state
}

const RAND_MAX: i32 = i32::MAX;
const ULONG_MAX: u32 = u32::MAX;

fn srand(env: &mut Environment, seed: u32) {
    env.libc_state.stdlib.rand = seed;
}
fn rand(env: &mut Environment) -> i32 {
    env.libc_state.stdlib.rand = prng(env.libc_state.stdlib.rand);
    (env.libc_state.stdlib.rand as i32) & RAND_MAX
}

// BSD's "better" random number generator, with an implementation that is not
// actually better.
fn srandom(env: &mut Environment, seed: u32) {
    env.libc_state.stdlib.random = seed;
}
fn random(env: &mut Environment) -> i32 {
    env.libc_state.stdlib.random = prng(env.libc_state.stdlib.random);
    (env.libc_state.stdlib.random as i32) & RAND_MAX
}

fn arc4random(env: &mut Environment) -> u32 {
    env.libc_state.stdlib.arc4random = prng(env.libc_state.stdlib.arc4random);
    env.libc_state.stdlib.arc4random
}

fn getenv(env: &mut Environment, name: ConstPtr<u8>) -> MutPtr<u8> {
    let name_cstr = env.mem.cstr_at(name);
    if name_cstr == b"MONO_LOG_LEVEL" {
        return env.mem.alloc_and_write_cstr(b"debug");
    }
    // TODO: Provide all the system environment variables an app might expect to
    // find. Currently the only environment variables that can be found are
    // those put there by the app (Crash Bandicoot Nitro Kart 3D uses this).
    let Some(&value) = env.libc_state.stdlib.env.get(name_cstr) else {
        log!(
            "Warning: getenv() for {:?} ({:?}) unhandled",
            name,
            std::str::from_utf8(name_cstr)
        );
        return Ptr::null();
    };
    log_dbg!(
        "getenv({:?} ({:?})) => {:?} ({:?})",
        name,
        name_cstr,
        value,
        env.mem.cstr_at_utf8(value),
    );
    // Caller should not modify the result
    value
}
fn setenv(env: &mut Environment, name: ConstPtr<u8>, value: ConstPtr<u8>, overwrite: i32) -> i32 {
    let name_cstr = env.mem.cstr_at(name);
    if let Some(&existing) = env.libc_state.stdlib.env.get(name_cstr) {
        if overwrite == 0 {
            return 0; // success
        }
        env.mem.free(existing.cast());
    };
    let value = super::string::strdup(env, value);
    let name_cstr = env.mem.cstr_at(name); // reborrow
    env.libc_state.stdlib.env.insert(name_cstr.to_vec(), value);
    log_dbg!(
        "Stored new value {:?} ({:?}) for environment variable {:?}",
        value,
        env.mem.cstr_at_utf8(value),
        std::str::from_utf8(name_cstr),
    );
    0 // success
}
fn unsetenv(env: &mut Environment, name: ConstPtr<u8>) -> i32 {
    let name_cstr = env.mem.cstr_at(name);
    assert!(env.libc_state.stdlib.env.get(name_cstr).is_none());
    0 // success
}

fn exit(_env: &mut Environment, exit_code: i32) {
    echo!("App called exit(), exiting.");
    std::process::exit(exit_code);
}

fn bsearch(
    env: &mut Environment,
    key: ConstVoidPtr,
    items: ConstVoidPtr,
    item_count: GuestUSize,
    item_size: GuestUSize,
    compare_callback: GuestFunction, // (*int)(const void*, const void*)
) -> ConstVoidPtr {
    log_dbg!(
        "binary search for {:?} in {} items of size {:#x} starting at {:?}",
        key,
        item_count,
        item_size,
        items
    );
    let mut low = 0;
    let mut len = item_count;
    while len > 0 {
        let half_len = len / 2;
        let item: ConstVoidPtr = (items.cast::<u8>() + item_size * (low + half_len)).cast();
        // key must be first argument
        let cmp_result: i32 = compare_callback.call_from_host(env, (key, item));
        (low, len) = match cmp_result.signum() {
            0 => {
                log_dbg!("=> {:?}", item);
                return item;
            }
            1 => (low + half_len + 1, len - half_len - 1),
            -1 => (low, half_len),
            _ => unreachable!(),
        }
    }
    log_dbg!("=> NULL (not found)");
    Ptr::null()
}

fn strtod(env: &mut Environment, nptr: ConstPtr<u8>, endptr: MutPtr<MutPtr<u8>>) -> f64 {
    log!("strtod nptr {}", env.mem.cstr_at_utf8(nptr).unwrap());
    let (d, len) = atof_inner(env, nptr).unwrap_or((0.0, 0));
    if !endptr.is_null() {
        env.mem.write(endptr, (nptr + len).cast_mut());
    }
    d
}

fn strtof(env: &mut Environment, nptr: ConstPtr<u8>, endptr: MutPtr<ConstPtr<u8>>) -> f32 {
    let (number, length) = atof_inner(env, nptr).unwrap_or((0.0, 0));
    if !endptr.is_null() {
        env.mem.write(endptr, nptr + length);
    }
    number as f32
}

fn realpath(env: &mut Environment, file_name: ConstPtr<u8>, resolve_name: MutPtr<u8>) -> MutPtr<u8> {
    assert!(!resolve_name.is_null());

    let file_name_str = env.mem.cstr_at_utf8(file_name).unwrap();
    log_dbg!("realpath file name {}", file_name_str);
    // assert!(!file_name_str.contains("/.") && file_name_str.as_bytes()[0] != b'.');
    if file_name_str.as_bytes()[0] == b'/' {
        strcpy(env, resolve_name, file_name);
        return resolve_name;
    }

    let cwd_ptr = getcwd(env, Ptr::null(), 0);
    let cwd_len = strlen(env, cwd_ptr.cast_const());

    strcpy(env, resolve_name, cwd_ptr.cast_const());
    env.mem.write(resolve_name + cwd_len, b'/');
    strcpy(env, resolve_name + cwd_len + 1, file_name);

    let resolve_name_str = env.mem.cstr_at_utf8(resolve_name).unwrap();
    log_dbg!("realpath resolve name {}", resolve_name_str);

    resolve_name
}

fn sched_yield(env: &mut Environment) -> i32 {
    0
}

fn mbstowcs(env: &mut Environment, pwcs: MutPtr<wchar_t>, s: ConstPtr<u8>, n: GuestUSize) -> GuestUSize {
    // TODO: assert C locale
    let size = strlen(env, s);
    let to_write = size.min(n);
    for i in 0..to_write {
        let c = env.mem.read(s + i);
        env.mem.write(pwcs + i, c as wchar_t);
    }
    if to_write < n {
        env.mem.write(pwcs + to_write, wchar_t::default());
    }
    to_write
}

// size_t
//      wcstombs(char *restrict s, const wchar_t *restrict pwcs, size_t n);
fn wcstombs(env: &mut Environment, s: ConstPtr<u8>, pwcs: MutPtr<wchar_t>, n: GuestUSize) -> GuestUSize {
    if n == 0 {
        return 0;
    }
    let x = env.mem.wcstr_at(pwcs);
    let len: GuestUSize = x.bytes().len() as GuestUSize;
    log!("wcstombs '{}', len {}, n {}", x, len, n);
    env.mem.bytes_at_mut(s.cast_mut(), n).write(x.as_bytes()).unwrap();
    if len < n {
        env.mem.write((s + len).cast_mut(), b'\0');
    }
    len
}

fn setlocale(env: &mut Environment, _category: i32, locale: ConstPtr<u8>) -> MutPtr<u8> {
    // assert_eq!(category, 0); // LC_ALL
    if !locale.is_null() {
        assert_eq!("C", env.mem.cstr_at_utf8(locale).unwrap());
        locale.cast_mut()
    } else {
        env.mem.alloc_and_write_cstr(b"C")
    }
}

pub fn strtoul(env: &mut Environment, str: ConstPtr<u8>, endptr: MutPtr<MutPtr<u8>>, base: i32) -> u32 {
    let s = env.mem.cstr_at_utf8(str).unwrap();
    log_dbg!("strtoul '{}'", s);
    assert_eq!(base, 16);
    let without_prefix = s.trim_start_matches("0x");
    let res = u32::from_str_radix(without_prefix, 16).unwrap_or(ULONG_MAX);
    if !endptr.is_null() {
        let len: GuestUSize = s.len().try_into().unwrap();
        env.mem.write(endptr, (str + len).cast_mut());
    }
    res
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(malloc(_)),
    export_c_func!(calloc(_, _)),
    export_c_func!(realloc(_, _)),
    export_c_func!(free(_)),
    export_c_func!(atexit(_)),
    export_c_func!(atoi(_)),
    export_c_func!(atol(_)),
    export_c_func!(atof(_)),
    export_c_func!(srand(_)),
    export_c_func!(rand()),
    export_c_func!(srandom(_)),
    export_c_func!(random()),
    export_c_func!(arc4random()),
    export_c_func!(getenv(_)),
    export_c_func!(setenv(_, _, _)),
    export_c_func!(unsetenv(_)),
    export_c_func!(exit(_)),
    export_c_func!(bsearch(_, _, _, _, _)),
    export_c_func!(strtof(_, _)),
    export_c_func!(strtod(_, _)),
    export_c_func2!("_realpath$DARWIN_EXTSN", realpath(_, _)),
    export_c_func!(sched_yield()),
    export_c_func!(mbstowcs(_, _, _)),
    export_c_func!(wcstombs(_, _, _)),
    export_c_func!(setlocale(_, _)),
    export_c_func!(strtoul(_, _, _)),
];

/// Returns a tuple containing the parsed number and the length of the number in
/// the string
pub fn atof_inner(env: &mut Environment, s: ConstPtr<u8>) -> Result<(f64, u32), <f64 as FromStr>::Err> {
    // atof() is similar to atoi().
    // FIXME: no C99 hexfloat, INF, NAN support
    let start = skip_whitespace(env, s);
    let whitespace_len = Ptr::to_bits(start) - Ptr::to_bits(s);
    let mut len = 0;
    let maybe_sign = env.mem.read(start + len);
    if maybe_sign == b'+' || maybe_sign == b'-' || maybe_sign.is_ascii_digit() {
        len += 1;
    }
    while env.mem.read(start + len).is_ascii_digit() {
        len += 1;
    }
    if env.mem.read(start + len) == b'.' {
        len += 1;
        while env.mem.read(start + len).is_ascii_digit() {
            len += 1;
        }
    }
    if env.mem.read(start + len).to_ascii_lowercase() == b'e' {
        len += 1;
        let maybe_sign = env.mem.read(start + len);
        if maybe_sign == b'+' || maybe_sign == b'-' || maybe_sign.is_ascii_digit() {
            len += 1;
        }
        while env.mem.read(start + len).is_ascii_digit() {
            len += 1;
        }
    }

    let s = std::str::from_utf8(env.mem.bytes_at(start, len)).unwrap();
    s.parse().map(|result| (result, whitespace_len + len))
}
