/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `printf` function family. The implementation is also used by `NSLog`.

use crate::abi::{GuestArg, VAList};
use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, GuestUSize, Mem, MutPtr, MutVoidPtr};
use crate::Environment;
use std::io::Write;
use crate::cpu::Cpu;

/// String formatting implementation for `printf` and `NSLog` function families.
///
/// `NS_LOG` is [true] for the `NSLog` format string type, or [false] for the
/// `printf` format string type.
///
/// `get_format_char` is a callback that returns the byte at a given index in
/// the format string, or `'\0'` if the index is one past the last byte.
pub fn printf_inner<const NS_LOG: bool, F: Fn(&Mem, GuestUSize) -> u8>(
    env: &mut Environment,
    get_format_char: F,
    mut args: VAList,
) -> Vec<u8> {
    let mut res = Vec::<u8>::new();

    let mut format_char_idx = 0;

    loop {
        let c = get_format_char(&env.mem, format_char_idx);
        format_char_idx += 1;

        if c == b'\0' {
            break;
        }
        if c != b'%' {
            res.push(c);
            continue;
        }

        let mut pad_char = if get_format_char(&env.mem, format_char_idx) == b'0' {
            format_char_idx += 1;
            '0'
        } else {
            ' '
        };
        if get_format_char(&env.mem, format_char_idx) == b'.' {
            format_char_idx += 1;
            pad_char = '0';
        }
        let pad_width = {
            let mut pad_width = 0;
            while let c @ b'0'..=b'9' = get_format_char(&env.mem, format_char_idx) {
                pad_width = pad_width * 10 + (c - b'0') as usize;
                format_char_idx += 1;
            }
            pad_width
        };

        let specifier = get_format_char(&env.mem, format_char_idx);
        format_char_idx += 1;

        assert!(specifier != b'\0');
        if specifier == b'%' {
            res.push(b'%');
            continue;
        }

        match specifier {
            b's' => {
                let c_string: ConstPtr<u8> = args.next(env);
                assert!(pad_char == ' ' && pad_width == 0); // TODO
                if !c_string.is_null() {
                    res.extend_from_slice(env.mem.cstr_at(c_string));
                }
            }
            b'd' | b'i' | b'u' => {
                let int: i64 = if specifier == b'u' {
                    let uint: u32 = args.next(env);
                    uint.into()
                } else {
                    let int: i32 = args.next(env);
                    int.into()
                };
                // TODO: avoid copy?
                if pad_width > 0 {
                    if pad_char == '0' {
                        res.extend_from_slice(format!("{:01$}", int, pad_width).as_bytes());
                    } else {
                        res.extend_from_slice(format!("{:1$}", int, pad_width).as_bytes());
                    }
                } else {
                    res.extend_from_slice(format!("{}", int).as_bytes());
                }
            }
            b'f' => {
                let float: f64 = args.next(env);
                // TODO: avoid copy?
                if pad_width > 0 {
                    if pad_char == '0' {
                        res.extend_from_slice(format!("{:01$}", float, pad_width).as_bytes());
                    } else {
                        res.extend_from_slice(format!("{:1$}", float, pad_width).as_bytes());
                    }
                } else {
                    res.extend_from_slice(format!("{}", float).as_bytes());
                }
            }
            b'x' => {
                let int: i64 = args.next(env);
                res.extend_from_slice(format!("{:x}", int).as_bytes());
            }
            // TODO: more specifiers
            _ => unimplemented!("Format character '{}'", specifier as char),
        }
    }

    log_dbg!("=> {:?}", std::str::from_utf8(&res));

    res
}

fn sscanf(env: &mut Environment, src: ConstPtr<u8>, format: ConstPtr<u8>, args: VAList) -> i32 {
    // TODO: implement
    0
}

fn vsnprintf(env: &mut Environment, dest: MutPtr<u8>, n: GuestUSize, format: ConstPtr<u8>, arg: MutVoidPtr) -> i32 {
    log_dbg!(
        "vsnprintf({:?} {:?} {:?})",
        dest,
        format,
        env.mem.cstr_at_utf8(format)
    );

    let mut fake_regs = [0u32; 16];
    fake_regs[Cpu::SP] = arg.to_bits();

    let va_list = VAList { reg_offset: 4, fake_regs: Some(fake_regs) };

    let res = printf_inner::<false, _>(env, |mem, idx| mem.read(format + idx), va_list);
    let middle = if ((n - 1) as usize) < res.len() {
        &res[..(n - 1) as usize]
    } else {
        &res[..]
    };

    let dest_slice = env
        .mem
        .bytes_at_mut(dest, n.try_into().unwrap());
    for (i, &byte) in middle.iter().chain(b"\0".iter()).enumerate() {
        dest_slice[i] = byte;
    }

    res.len().try_into().unwrap()
}

fn sprintf(env: &mut Environment, dest: MutPtr<u8>, format: ConstPtr<u8>, args: VAList) -> i32 {
    log_dbg!(
        "sprintf({:?}, {:?} ({:?}), ...)",
        dest,
        format,
        env.mem.cstr_at_utf8(format)
    );

    let res = printf_inner::<false, _>(env, |mem, idx| mem.read(format + idx), args);

    let dest_slice = env
        .mem
        .bytes_at_mut(dest, (res.len() + 1).try_into().unwrap());
    for (i, &byte) in res.iter().chain(b"\0".iter()).enumerate() {
        dest_slice[i] = byte;
    }

    res.len().try_into().unwrap()
}

fn printf(env: &mut Environment, format: ConstPtr<u8>, args: VAList) -> i32 {
    log_dbg!(
        "printf({:?} ({:?}), ...)",
        format,
        env.mem.cstr_at_utf8(format)
    );

    let res = printf_inner::<false, _>(env, |mem, idx| mem.read(format + idx), args);
    // TODO: I/O error handling
    let _ = std::io::stdout().write_all(&res);
    res.len().try_into().unwrap()
}

// TODO: more printf variants

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(sscanf(_, _, _)),
    export_c_func!(vsnprintf(_, _, _, _)),
    export_c_func!(sprintf(_, _, _)),
    export_c_func!(printf(_, _)),
];
