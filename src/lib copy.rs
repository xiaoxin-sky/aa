use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use std::{
  collections::HashMap,
  ffi::{c_char, c_int, CStr, CString},
  sync::{Arc, Mutex},
};

use libloading::{Library, Symbol};
use napi::{bindgen_prelude::*, threadsafe_function::ThreadsafeFunction};
use napi_derive::napi;
use once_cell::sync::Lazy;

static GLOBAL_TSFN: Lazy<Arc<Mutex<Option<ThreadsafeFunction<FnArgs<(String, String)>, u32>>>>> =
  Lazy::new(|| Arc::new(Mutex::new(None)));

static GLOBAL_MESSAGE_HANDLE_FN: Lazy<
  Arc<Mutex<Option<ThreadsafeFunction<FnArgs<(String, String)>, ()>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(None)));

static GLOBAL_DISCONNECT_HANDLE_FN: Lazy<
  Arc<Mutex<Option<ThreadsafeFunction<FnArgs<(String, String)>, ()>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(None)));

static GLOBAL_DISCONNECT_HANDLERS: Lazy<
  Mutex<HashMap<u32, ThreadsafeFunction<FnArgs<(String, String)>, ()>>>,
> = Lazy::new(|| Mutex::new(HashMap::new()));

type FnGetSystemInfo =
  unsafe extern "C" fn(*mut c_char, *mut c_int, *mut c_char, *mut c_int) -> c_int;

type AsyncSocketClientConnect = unsafe extern "C" fn(
  host: *const c_char,
  port: *const c_char,
  token: *const c_char,
  callback: extern "C" fn(*const c_char, *const c_char),
) -> c_int;

type AsyncSocketClientSetMessageHandler =
  unsafe extern "C" fn(callback: extern "C" fn(*const c_char, *const c_char)) -> c_int;

type AsyncSocketClientSetDisConnectionStatusCallback = unsafe extern "C" fn(
  candle_id: u32,
  callback: extern "C" fn(*const c_char, *const c_char),
) -> c_int;
#[napi]
pub struct TcpClient {
  lib: Library,
}

#[napi]
impl TcpClient {
  #[napi(constructor)]
  pub fn new(lib_path: String) -> Result<Self> {
    let lib = unsafe { Library::new(lib_path).map_err(|e| Error::from_reason(e.to_string()))? };
    Ok(TcpClient { lib })
  }

  #[napi]
  pub fn create(&self) -> Result<()> {
    unsafe {
      let func: Symbol<unsafe extern "C" fn()> =
        self.lib.get(b"AsyncSocketClient_create\0").unwrap();
      func();
    }
    Ok(())
  }

  #[napi]
  pub fn send(&self, id: u32, msg: String) -> Result<()> {
    unsafe {
      let func: Symbol<unsafe extern "C" fn(u32, *const i8)> =
        self.lib.get(b"AsyncSocketClient_send\0").unwrap();
      func(id, std::ffi::CString::new(msg)?.as_ptr());
    }
    Ok(())
  }

  #[napi]
  pub fn disconnect(&self, id: u32) -> Result<()> {
    unsafe {
      let func: Symbol<unsafe extern "C" fn(u32)> =
        self.lib.get(b"AsyncSocketClient_disconnect\0").unwrap();
      func(id);
    }
    Ok(())
  }

  #[napi]
  pub fn cleanup(&self) -> Result<()> {
    unsafe {
      let func: Symbol<unsafe extern "C" fn()> = self.lib.get(b"Cleanup\0").unwrap();
      func();
    }
    Ok(())
  }

  #[napi]
  pub fn get_system_info(&self) -> Result<(i32, String, String)> {
    let mut out1 = vec![0u8; 1024];
    let mut out2 = vec![0u8; 512];

    let func: Symbol<FnGetSystemInfo> =
      unsafe { self.lib.get(b"AsyncSocketClient_GetSystemInfo\0").unwrap() };

    let res = unsafe {
      func(
        out1.as_mut_ptr() as *mut c_char,
        std::ptr::null_mut(),
        out2.as_mut_ptr() as *mut c_char,
        std::ptr::null_mut(),
      )
    };
    Ok((
      res,
      String::from_utf8_lossy(&out1).to_string(),
      String::from_utf8_lossy(&out2).to_string(),
    ))
  }

  #[napi]
  pub fn connect(
    &self,
    portal: String,
    host: String,
    port: String,
    callback: Function<FnArgs<(String, String)>, u32>,
  ) -> Result<i32> {
    let tsfn = callback
      .build_threadsafe_function()
      .callee_handled()
      .build()
      .unwrap();

    {
      let mut guard = GLOBAL_TSFN.lock().unwrap();
      *guard = Some(tsfn);
    }
    extern "C" fn my_connect_callback(candle_id: *const c_char, message: *const c_char) {
      let id = unsafe {
        std::ffi::CStr::from_ptr(candle_id)
          .to_string_lossy()
          .into_owned()
      };
      let msg = unsafe {
        std::ffi::CStr::from_ptr(message)
          .to_string_lossy()
          .into_owned()
      };

      if let Ok(mut guard) = GLOBAL_TSFN.lock() {
        if let Some(fn_handle) = guard.take() {
          fn_handle.call(
            Ok(FnArgs { data: (id, msg) }),
            ThreadsafeFunctionCallMode::Blocking,
          );
        }
      }
    }

    let func: Symbol<AsyncSocketClientConnect> =
      unsafe { self.lib.get(b"AsyncSocketClient_connect\0").unwrap() };
    let res = unsafe {
      func(
        CString::new(portal)?.as_ptr(),
        CString::new(host)?.as_ptr(),
        CString::new(port)?.as_ptr(),
        my_connect_callback,
      )
    };
    Ok(res)
  }

  #[napi]
  pub fn set_message_handle(&self, callback: Function<FnArgs<(String, String)>, ()>) -> Result<()> {
    let tsfn = callback
      .build_threadsafe_function()
      .callee_handled()
      .build()
      .unwrap();
    {
      let mut guard = GLOBAL_MESSAGE_HANDLE_FN.lock().unwrap();
      *guard = Some(tsfn);
    }

    extern "C" fn my_message_handler_callback(candle_id: *const c_char, message: *const c_char) {
      let id = unsafe { CStr::from_ptr(candle_id).to_string_lossy().into_owned() };
      let msg = unsafe { CStr::from_ptr(message).to_string_lossy().into_owned() };

      if let Ok(guard) = GLOBAL_MESSAGE_HANDLE_FN.lock() {
        if let Some(fn_handle) = guard.as_ref() {
          fn_handle.call(
            Ok(FnArgs { data: (id, msg) }),
            ThreadsafeFunctionCallMode::NonBlocking,
          );
        }
      }
    }
    let func: Symbol<AsyncSocketClientSetMessageHandler> = unsafe {
      self
        .lib
        .get(b"AsyncSocketClient_setMessageHandler\0")
        .unwrap()
    };
    unsafe { func(my_message_handler_callback) };
    Ok(())
  }

  #[napi]
  pub fn set_disconnect_handler(
    &self,
    candle_id: u32,
    callback: Function<FnArgs<(String, String)>, ()>,
  ) {
    let tsfn: ThreadsafeFunction<FnArgs<(String, String)>, ()> = callback
      .build_threadsafe_function()
      .callee_handled()
      .build()
      .unwrap();

    {
      let mut guard = GLOBAL_DISCONNECT_HANDLERS.lock().unwrap();
      guard.insert(candle_id, tsfn);
    }

    extern "C" fn my_disconnect_handler_callback(candle_id: *const c_char, message: *const c_char) {
      let id = unsafe { CStr::from_ptr(candle_id).to_string_lossy().into_owned() };
      let msg = unsafe { CStr::from_ptr(message).to_string_lossy().into_owned() };

      let id_str = id.clone();

      if let Ok(id) = id.parse::<u32>() {
        if let Ok(mut guard) = GLOBAL_DISCONNECT_HANDLERS.lock() {
          // 取出并移除 ThreadsafeFunction
          if let Some(tsfn) = guard.remove(&id) {
            tsfn.call(
              Ok(FnArgs {
                data: (id_str, msg),
              }),
              ThreadsafeFunctionCallMode::NonBlocking,
            );
          }
        }
      }
    }

    let func: Symbol<AsyncSocketClientSetDisConnectionStatusCallback> = unsafe {
      self
        .lib
        .get(b"AsyncSocketClient_setDisConnectionStatusCallback\0")
        .unwrap()
    };

    unsafe { func(candle_id, my_disconnect_handler_callback) };
  }
}
