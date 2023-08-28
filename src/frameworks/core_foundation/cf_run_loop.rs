/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFRunLoop`.
//!
//! This is not even toll-free bridged to `NSRunLoop` in Apple's implementation,
//! but here it is the same type.

use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::objc::{Class, id, msg, msg_class, nil};
use crate::Environment;
use crate::frameworks::core_foundation::cf_allocator::CFAllocatorRef;
use crate::frameworks::core_foundation::CFIndex;
use crate::frameworks::core_foundation::time::{CFAbsoluteTime, CFTimeInterval};
use crate::mem::{MutVoidPtr, Ptr};

pub type CFRunLoopRef = super::CFTypeRef;
pub type CFRunLoopMode = super::cf_string::CFStringRef;

pub type CFRunLoopTimerRef = super::CFTypeRef;
pub type CFOptionFlags = u64;

fn CFRunLoopGetCurrent(env: &mut Environment) -> CFRunLoopRef {
    msg_class![env; NSRunLoop currentRunLoop]
}

pub fn CFRunLoopGetMain(env: &mut Environment) -> CFRunLoopRef {
    msg_class![env; NSRunLoop mainRunLoop]
}

// CFRunLoopTimerRef CFRunLoopTimerCreate(
// CFAllocatorRef allocator, CFAbsoluteTime fireDate, CFTimeInterval interval,
// CFOptionFlags flags, CFIndex order, CFRunLoopTimerCallBack callout, CFRunLoopTimerContext *context
// )

// typedef void (*CFRunLoopTimerCallBack)(CFRunLoopTimerRef timer, void *info)
fn CFRunLoopTimerCreate(
    env: &mut Environment, _allocator: CFAllocatorRef, fireDate: CFAbsoluteTime,
    interval: CFTimeInterval, flags: CFOptionFlags, _order: CFIndex, callout: GuestFunction,
    context: MutVoidPtr
) -> CFRunLoopTimerRef {
    assert_eq!(flags, 0);
    assert!(context.is_null());

    let fake_target: id = msg_class![env; FakeCFTimerTarget alloc];
    let fake_target: id = msg![env; fake_target initWithCallout:callout];

    let selector = env.objc.lookup_selector("timerFireMethod:").unwrap();

    let repeats = interval > 0.0;
    let timer =
        msg_class![env; NSTimer timerWithTimeInterval:interval
                                               target:fake_target
                                             selector:selector
                                             userInfo:nil
                                              repeats:repeats];

    timer
}

// void CFRunLoopAddTimer(CFRunLoopRef rl, CFRunLoopTimerRef timer, CFRunLoopMode mode);
fn CFRunLoopAddTimer(env: &mut Environment, rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFRunLoopMode) {
    let rl_class: Class = msg![env; rl class];
    assert_eq!(rl_class, env.objc.get_known_class("NSRunLoop", &mut env.mem));

    let timer_class: Class = msg![env; timer class];
    assert_eq!(timer_class, env.objc.get_known_class("NSTimer", &mut env.mem));

    () = msg![env; rl addTimer:timer forMode:mode];
}

pub const kCFRunLoopCommonModes: &str = "kCFRunLoopCommonModes";
pub const kCFRunLoopDefaultMode: &str = "kCFRunLoopDefaultMode";
pub const kCFBundleExecutableKey: &str = "kCFBundleExecutableKey";

pub const CONSTANTS: ConstantExports = &[
    (
        "_kCFRunLoopCommonModes",
        HostConstant::NSString(kCFRunLoopCommonModes),
    ),
    (
        "_kCFRunLoopDefaultMode",
        HostConstant::NSString(kCFRunLoopDefaultMode),
    ),
    (
        "_kCFBundleExecutableKey",
        HostConstant::NSString(kCFBundleExecutableKey),
    ),
];

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFRunLoopGetCurrent()),
    export_c_func!(CFRunLoopGetMain()),
    export_c_func!(CFRunLoopTimerCreate(_, _, _, _, _, _, _)),
    export_c_func!(CFRunLoopAddTimer(_, _, _)),
];
