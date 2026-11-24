//! 运行时加载系统库（显卡驱动、PipeWire、PulseAudio）。程序本身不链接它们：没装就加载失败、跳过这个功能。
//! 加载后不卸载（有的驱动库卸载时会出问题）。

use std::ffi::{c_void, CStr};

pub struct Lib {
    handle: *mut c_void,
    pub name: String,
}

// dlopen 句柄可以跨线程用
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Lib {
    /// 按顺序试这些库名，返回第一个能加载的。
    pub fn open(names: &[&str]) -> Result<Self, String> {
        let mut last = String::new();
        for n in names {
            let c = std::ffi::CString::new(*n).map_err(|e| e.to_string())?;
            let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !h.is_null() {
                return Ok(Self { handle: h, name: n.to_string() });
            }
            last = unsafe {
                let e = libc::dlerror();
                if e.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(e).to_string_lossy().into_owned()
                }
            };
        }
        Err(if last.is_empty() { format!("找不到 {}", names.join(" / ")) } else { last })
    }

    /// 取函数地址。`T` 必须是函数指针类型。
    ///
    /// # Safety
    /// 调用方保证 `T` 与库里的函数签名一致。
    pub unsafe fn sym<T: Copy>(&self, name: &CStr) -> Result<T, String> {
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        let p = libc::dlsym(self.handle, name.as_ptr());
        if p.is_null() {
            return Err(format!("{} 里没有 {}", self.name, name.to_string_lossy()));
        }
        Ok(std::mem::transmute_copy(&p))
    }
}

/// 声明一组从同一个库取的函数：生成结构体与 `load(&Lib)`。
#[macro_export]
macro_rules! dl_fns {
    ($vis:vis struct $name:ident { $( $f:ident : fn($($a:ty),*) $(-> $r:ty)? ; )* }) => {
        #[allow(non_snake_case, dead_code)]
        $vis struct $name {
            $( pub $f: unsafe extern "C" fn($($a),*) $(-> $r)?, )*
        }
        impl $name {
            #[allow(non_snake_case)]
            pub fn load(lib: &$crate::desktop::imp::encode::dl::Lib) -> Result<Self, String> {
                unsafe {
                    Ok(Self {
                        $( $f: lib.sym(&std::ffi::CString::new(stringify!($f)).unwrap())?, )*
                    })
                }
            }
        }
    };
}
