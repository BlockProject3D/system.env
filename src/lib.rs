// Copyright (c) 2022, BlockProject 3D
//
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without modification,
// are permitted provided that the following conditions are met:
//
//     * Redistributions of source code must retain the above copyright notice,
//       this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright notice,
//       this list of conditions and the following disclaimer in the documentation
//       and/or other materials provided with the distribution.
//     * Neither the name of BlockProject 3D nor the names of its contributors
//       may be used to endorse or promote products derived from this software
//       without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
// CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
// PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
// LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
// NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static PATHS: Lazy<Mutex<Vec<PathBuf>>> = Lazy::new(|| Mutex::new(Vec::new()));
static ENV_CACHE: Lazy<Mutex<HashMap<OsString, Option<OsString>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Adds a new override path.
///
/// If the path is already added, nothing happens. If the path is not already present, the
/// requested path is cloned and inserted in the global path list.
///
/// Additionally, when a new path is added, the function invalidates the cache to let a chance
/// to the getters to read the new override.
///
/// **Note: This is a slow function with allocations, locks and linear search.**
///
/// This is best called when initializing the application.
///
/// # Panics
///
/// The function panics if the path does not point to a file.
pub fn add_override_path(path: &Path) {
    if !path.is_file() {
        panic!("Cannot add non-file environment override path!")
    }
    let mut lock = PATHS.lock().unwrap();
    if lock.iter().any(|p| p == path) {
        return;
    }
    lock.push(path.into());
    let mut lock1 = ENV_CACHE.lock().unwrap();
    lock1.clear();
}

fn insert_key_value(
    cache: &mut HashMap<OsString, Option<OsString>>,
    key: impl AsRef<OsStr>,
    value: impl AsRef<OsStr>,
) {
    if value.as_ref().is_empty() {
        cache.insert(key.as_ref().into(), None);
    } else {
        cache.insert(key.as_ref().into(), Some(value.as_ref().into()));
    }
}

#[cfg(windows)]
fn windows_path(cache: &mut HashMap<OsString, Option<OsString>>, data: &[u8], pos: usize) -> bool {
    let key = match std::str::from_utf8(&data[..pos]) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let value = match std::str::from_utf8(&data[pos + 1..]) {
        Ok(v) => v,
        Err(_) => return false,
    };
    insert_key_value(cache, key, value);
    true
}

/// Gets the content of an environment variable.
///
/// Returns None if the variable does not exist.
///
/// **Note: for optimization reasons, the functions caches values.**
///
/// The cost of this function is amortized O(1) (thanks to the cache). Once a value is loaded it's
/// cached to avoid re-loading it. When a value is not loaded the cost of this function is O(nm)
/// with n the number of items in the override path list and m the number of lines in each override
/// file.
pub fn get_os<T: AsRef<OsStr>>(name: T) -> Option<OsString> {
    let mut cache = ENV_CACHE.lock().unwrap();
    {
        // Attempt to pull from the cache.
        if let Some(val) = cache.get(name.as_ref()) {
            return val.clone();
        }
    }
    {
        // Value is not in cache, try pulling from environment variables.
        if let Some(val) = std::env::var_os(name.as_ref()) {
            cache.insert(name.as_ref().into(), Some(val));
        }
        if let Some(val) = cache.get(name.as_ref()) {
            return val.clone();
        }
    }
    {
        // Value is still not in cache, try pulling from the override file list.
        let lock = PATHS.lock().unwrap();
        for v in &*lock {
            let file = match File::open(v) {
                Ok(v) => BufReader::new(v),
                Err(_) => continue,
            };
            for v in file.split(b'\n') {
                let data = match v {
                    Ok(v) => v,
                    Err(_) => break, // If an IO error has occurred, skip loading the file
                                     // completely.
                };
                let pos = match data.iter().position(|v| *v == b'=') {
                    Some(v) => v,
                    None => continue,
                };
                #[cfg(windows)]
                if !windows_path(&mut cache, &data, pos) {
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStrExt;
                    //Unix is better because it accepts constructing OsStr from a byte buffer.
                    let key = OsStr::from_bytes(&data[..pos]);
                    let value = OsStr::from_bytes(&data[pos + 1..]);
                    insert_key_value(&mut cache, key, value);
                }
                if let Some(val) = cache.get(name.as_ref()) {
                    return val.clone();
                }
            }
        }
    }
    // Everything failed; just place a None in the cache and assume the variable does not exist.
    cache.insert(name.as_ref().into(), None);
    None
}

/// Gets the content of an environment variable.
///
/// Returns None if the variable does not exist or is not valid UTF-8.
///
/// **Note: for optimization reasons, the functions caches values.**
///
/// The cost of this function is amortized O(1) (thanks to the cache). Once a value is loaded it's
/// cached to avoid re-loading it. When a value is not loaded the cost of this function is O(nm)
/// with n the number of items in the override path list and m the number of lines in each override
/// file.
pub fn get<T: AsRef<OsStr>>(name: T) -> Option<String> {
    get_os(name).and_then(|v| v.into_string().ok())
}

/// Gets a boolean environment variable.
///
/// Returns None if the variable does not exist or the format is unrecognized.
///
/// **Note: for optimization reasons, the functions caches values.**
///
/// The cost of this function is amortized O(1) (thanks to the cache). Once a value is loaded it's
/// cached to avoid re-loading it. When a value is not loaded the cost of this function is O(nm)
/// with n the number of items in the override path list and m the number of lines in each override
/// file.
pub fn get_bool<T: AsRef<OsStr>>(name: T) -> Option<bool> {
    match &*get(name)? {
        "off" | "OFF" | "FALSE" | "false" | "0" => Some(false),
        "on" | "ON" | "TRUE" | "true" | "1" => Some(true),
        _ => None,
    }
}
