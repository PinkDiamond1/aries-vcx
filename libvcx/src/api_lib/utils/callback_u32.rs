use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Mutex;

use libc::c_char;

use aries_vcx::indy_sys::CommandHandle;

use crate::api_lib::utils::callback::{build_buf, build_string, get_cb};

lazy_static! {
    pub static ref CALLBACKS_U32: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32) + Send>>> = Default::default();
    pub static ref CALLBACKS_U32_U32: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, u32) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_I32: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, i32) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_STR: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, Option<String>) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_U32_STR: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, u32, Option<String>) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_STR_STR: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, Option<String>, Option<String>) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_BOOL: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, bool) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_BIN: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, Vec<u8>) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_OPTSTR_BIN: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, Option<String>, Vec<u8>) + Send>>> =
        Default::default();
    pub static ref CALLBACKS_U32_U32_STR_STR_STR: Mutex<HashMap<CommandHandle, Box<dyn FnMut(u32, u32, Option<String>, Option<String>, Option<String>) + Send>>> =
        Default::default();
}

pub extern "C" fn call_cb_u32(command_handle: CommandHandle, arg1: u32) {
    let cb = get_cb(command_handle, CALLBACKS_U32.deref());
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1)
    }
}

pub extern "C" fn call_cb_u32_u32(command_handle: CommandHandle, arg1: u32, arg2: u32) {
    let cb = get_cb(command_handle, CALLBACKS_U32_U32.deref());
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, arg2)
    }
}

pub extern "C" fn call_cb_u32_u32_str(command_handle: CommandHandle, arg1: u32, arg2: u32, arg3: *const c_char) {
    let cb = get_cb(command_handle, CALLBACKS_U32_U32_STR.deref());
    let str1 = build_string(arg3);
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, arg2, str1)
    }
}

pub extern "C" fn call_cb_u32_i32(command_handle: CommandHandle, arg1: u32, arg2: i32) {
    let cb = get_cb(command_handle, CALLBACKS_U32_I32.deref());
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, arg2)
    }
}

pub extern "C" fn call_cb_u32_str(command_handle: CommandHandle, arg1: u32, arg2: *const c_char) {
    let cb = get_cb(command_handle, CALLBACKS_U32_STR.deref());
    let str1 = build_string(arg2);
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, str1)
    }
}

pub extern "C" fn call_cb_u32_str_str(
    command_handle: CommandHandle,
    arg1: u32,
    arg2: *const c_char,
    arg3: *const c_char,
) {
    let cb = get_cb(command_handle, CALLBACKS_U32_STR_STR.deref());
    let str1 = build_string(arg2);
    let str2 = build_string(arg3);
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, str1, str2)
    }
}

pub extern "C" fn call_cb_u32_bool(command_handle: CommandHandle, arg1: u32, arg2: bool) {
    let cb = get_cb(command_handle, CALLBACKS_U32_BOOL.deref());
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, arg2)
    }
}

pub extern "C" fn call_cb_u32_bin(command_handle: CommandHandle, arg1: u32, buf: *const u8, len: u32) {
    let cb = get_cb(command_handle, CALLBACKS_U32_BIN.deref());
    let data = build_buf(buf, len);
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, data)
    }
}

pub extern "C" fn call_cb_u32_str_bin(
    command_handle: CommandHandle,
    arg1: u32,
    arg2: *const c_char,
    buf: *const u8,
    len: u32,
) {
    let cb = get_cb(command_handle, CALLBACKS_U32_OPTSTR_BIN.deref());
    let data = build_buf(buf, len);

    let str1 = build_string(arg2);

    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, str1, data)
    }
}

pub extern "C" fn call_cb_u32_u32_str_str_str(
    command_handle: CommandHandle,
    arg1: u32,
    arg2: u32,
    arg3: *const c_char,
    arg4: *const c_char,
    arg5: *const c_char,
) {
    let cb = get_cb(command_handle, CALLBACKS_U32_U32_STR_STR_STR.deref());
    let str1 = build_string(arg3);
    let str2 = build_string(arg4);
    let str3 = build_string(arg5);
    if let Some(mut cb_fn) = cb {
        cb_fn(arg1, arg2, str1, str2, str3)
    }
}

#[cfg(test)]
#[cfg(feature = "general_test")]
mod tests {
    use std::ffi::CString;

    use super::*;

    fn cstring(str_val: &String) -> CString {
        CString::new(str_val.clone()).unwrap()
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_build_string() {
        let test_str = "Journey before destination".to_string();

        let test = build_string(cstring(&test_str).as_ptr());
        assert!(test.is_some());
        assert_eq!(test_str, test.unwrap());
    }

    #[test]
    #[cfg(feature = "general_test")]
    fn test_get_cb() {
        let mutex_map: Mutex<HashMap<u32, Box<dyn FnMut(u32) + Send>>> = Default::default();
        assert!(get_cb(2123, &mutex_map).is_none());

        let closure: Box<dyn FnMut(u32) + Send> = Box::new(move |_| {});

        mutex_map.lock().unwrap().insert(2123, closure);
        let cb = get_cb(2123, &mutex_map);
        assert!(cb.is_some());
    }
}
