/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSBundle`.

use super::ns_array;
use super::ns_string;
use crate::bundle::Bundle;
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, ClassExports, HostObject,
};
use crate::Environment;

// Should be ISO 639-1 (or ISO 639-2) compliant
// TODO: complete this list or use some crate for mapping
const LANG_ID_TO_LANG_PROJ: &[(&str, &str)] = &[
    ("da", "Danish.lproj"),
    ("nl", "Dutch.lproj"),
    ("en", "English.lproj"),
    ("fi", "Finnish.lproj"),
    ("fr", "French.lproj"),
    ("de", "German.lproj"),
    ("it", "Italian.lproj"),
    ("ja", "Japanese.lproj"),
    ("no", "Norwegian.lproj"),
    ("es", "Spanish.lproj"),
    ("sv", "Swedish.lproj"),
];

#[derive(Default)]
pub struct State {
    main_bundle: Option<id>,
}

struct NSBundleHostObject {
    /// If this is [None], this is the main bundle's NSBundle instance and the
    /// [Bundle] is stored in [crate::Environment], not here.
    _bundle: Option<Bundle>,
    /// NSString with bundle path.
    bundle_path: id,
    /// NSURL with bundle path. [None] if not created yet.
    bundle_url: Option<id>,
    /// `NSDictionary*` for the `Info.plist` content. [None] if not created yet.
    info_dictionary: Option<id>,
}
impl HostObject for NSBundleHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSBundle: NSObject

+ (id)mainBundle {
    if let Some(bundle) = env.framework_state.foundation.ns_bundle.main_bundle {
        bundle
    } else {
        let bundle_path = env.bundle.bundle_path().as_str().to_string();
        let bundle_path = ns_string::from_rust_string(env, bundle_path);
        let host_object = NSBundleHostObject {
            _bundle: None,
            bundle_path,
            bundle_url: None,
            info_dictionary: None,
        };
        let new = env.objc.alloc_object(
            this,
            Box::new(host_object),
            &mut env.mem
        );
        env.framework_state.foundation.ns_bundle.main_bundle = Some(new);
        new
   }
}

- (())dealloc {
    let &NSBundleHostObject {
        _bundle: _,
        bundle_path: _, // FIXME?
        bundle_url,
        info_dictionary,
    } = env.objc.borrow(this);
    if let Some(bundle_url) = bundle_url {
        release(env, bundle_url);
    }
    if let Some(info_dictionary) = info_dictionary {
        release(env, info_dictionary);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

- (id)bundlePath {
    env.objc.borrow::<NSBundleHostObject>(this).bundle_path
}
- (id)bundleURL {
    if let Some(url) = env.objc.borrow::<NSBundleHostObject>(this).bundle_url {
        url
    } else {
        let bundle_path: id = msg![env; this bundlePath];
        let new: id = msg_class![env; NSURL alloc];
        let new: id = msg![env; new initFileURLWithPath:bundle_path];
        env.objc.borrow_mut::<NSBundleHostObject>(this).bundle_url = Some(new);
        new
    }
}

- (id)resourcePath {
    // This seems to be the same as the bundle path. The iPhone OS bundle
    // structure is a lot flatter than the macOS one.
    msg![env; this bundlePath]
}
- (id)resourceURL {
    // This seems to be the same as the bundle path. The iPhone OS bundle
    // structure is a lot flatter than the macOS one.
    msg![env; this bundleURL]
}

- (id)pathForResource:(id)name // NSString*
               ofType:(id)extension // NSString*
          inDirectory:(id)directory { // NSString*
    assert!(name != nil); // TODO

    // TODO: cache result of lookups

    let path = path_for_resource_helper(env, this, name, nil, directory, extension);
    if path != nil {
        return path
    }

    // Get preferred languages
    let langs: id = msg_class![env; NSLocale preferredLanguages];
    // TODO: iterate over all
    let lang: id = msg![env; langs objectAtIndex:0u32];
    let lang_code = ns_string::to_rust_string(env, lang); // TODO: avoid copy
    let lproj_name = match LANG_ID_TO_LANG_PROJ.iter().find(|&&(code, _)| code == lang_code) {
        Some(&(_, name)) => name,
        None => {
            log!("TODO: {:?} is not mapped to a language name, fallback to English", lang_code);
            "English.lproj"
        }
    };
    let lproj: id = ns_string::get_static_str(env, lproj_name);
    let localized_path = path_for_resource_helper(env, this, name, lproj, directory, extension);
    if localized_path != nil {
        return localized_path
    }

    // As a last resort, fallback to English
    // TODO: fallback to a development language (CFBundleDevelopmentRegion from Info.plist)
    let lproj: id = ns_string::get_static_str(env, "English.lproj");
    path_for_resource_helper(env, this, name, lproj, directory, extension)
}

- (id)pathsForResourcesOfType:(id)extension // NSString*
    inDirectory:(id)directory { // NSString*
    assert!(directory.is_null());
    let ext = ns_string::to_rust_string(env, extension);
    // let dir = ns_string::to_rust_string(env, directory);
    //log!("ext {}", ext);
    assert_eq!("xml", ext);
    let name = ns_string::from_rust_string(env, "worlds_list.xml".to_owned());
    let path = msg![env; this pathForResource:name ofType:extension];
    ns_array::from_vec(env, vec![path])
}

- (id)pathForResource:(id)name // NSString*
               ofType:(id)extension { // NSString*
    msg![env; this pathForResource:name ofType:extension inDirectory:nil]
}
- (id)URLForResource:(id)name // NSString*
       withExtension:(id)extension // NSString *
        subdirectory:(id)subpath { // NSString *
   let path_string: id = msg![env; this pathForResource:name
                                                 ofType:extension
                                            inDirectory:subpath];
   let path_url: id = msg_class![env; NSURL alloc];
   let path_url: id = msg![env; path_url initFileURLWithPath:path_string];
   autorelease(env, path_url)
}
- (id)URLForResource:(id)name // NSString*
       withExtension:(id)extension { // NSString *
   msg![env; this URLForResource:name withExtension:extension subdirectory:nil]
}

- (id)localizedStringForKey:(id)key
                      value:(id)value
                      table:(id)tableName {
    log!("localizedStringForKey '{}' '{}' '{}'",
            if key == nil { std::borrow::Cow::from("(null)") } else { ns_string::to_rust_string(env, key) },
            if value == nil { std::borrow::Cow::from("(null)") } else { ns_string::to_rust_string(env, value) },
            if tableName == nil { std::borrow::Cow::from("(null)") } else { ns_string::to_rust_string(env, tableName) }
    );
    if value == nil || ns_string::to_rust_string(env, value).len() == 0 {
        return key;
    }
    value
}

- (id)infoDictionary {
    let &NSBundleHostObject {
        bundle_path,
        info_dictionary,
        ..
    } = env.objc.borrow(this);
    if let Some(dict) = info_dictionary {
        return dict;
    }

    let plist_path = ns_string::get_static_str(env, "Info.plist");
    let plist_path: id = msg![env; bundle_path stringByAppendingPathComponent:plist_path];
    let dict: id = msg_class![env; NSDictionary alloc];
    let dict: id = msg![env; dict initWithContentsOfFile:plist_path];
    env.objc.borrow_mut::<NSBundleHostObject>(this).info_dictionary = Some(dict);
    dict
}

// TODO: constructors, more accessors

@end

};

fn path_for_resource_helper(
    env: &mut Environment,
    bundle: id,
    name: id,
    lproj: id,
    directory: id,
    extension: id,
) -> id {
    let mut path: id = msg![env; bundle resourcePath];
    if lproj != nil {
        path = msg![env; path stringByAppendingPathComponent:lproj];
    }
    if directory != nil {
        path = msg![env; path stringByAppendingPathComponent:directory];
    }
    path = msg![env; path stringByAppendingPathComponent:name];
    if extension != nil {
        path = msg![env; path stringByAppendingPathExtension:extension];
    }
    let file_manager: id = msg_class![env; NSFileManager defaultManager];
    let file_exists: bool = msg![env; file_manager fileExistsAtPath:path];
    if file_exists {
        return path;
    }
    nil
}
