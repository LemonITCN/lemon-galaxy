//! 安卓专用：登记 JavaVM 与 Application Context。
//!
//! cpal 的 AAudio 后端打开输出流后会经 ndk-context 调 Java 接口，没登记会 panic。
//! 应用在 core_init 之前调一次 `cn.lemonit.galaxy.core.rust.NativeInit.init(applicationContext)`。

use std::sync::atomic::{AtomicBool, Ordering};

static DONE: AtomicBool = AtomicBool::new(false);

/// # Safety
/// 由 JVM 调用，参数是合法的 JNIEnv 与 Context 引用。
#[no_mangle]
pub unsafe extern "system" fn Java_cn_lemonit_galaxy_core_rust_NativeInit_init(env: *mut jni_sys::JNIEnv, _class: jni_sys::jclass, context: jni_sys::jobject) {
    if env.is_null() || context.is_null() || DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    let (Some(get_vm), Some(new_global)) = ((**env).GetJavaVM, (**env).NewGlobalRef) else {
        DONE.store(false, Ordering::SeqCst);
        return;
    };
    let mut vm: *mut jni_sys::JavaVM = std::ptr::null_mut();
    if get_vm(env, &mut vm) != 0 || vm.is_null() {
        DONE.store(false, Ordering::SeqCst);
        return;
    }
    // 全局引用一直留着（Application Context 与进程同生命周期）
    let ctx = new_global(env, context);
    if ctx.is_null() {
        DONE.store(false, Ordering::SeqCst);
        return;
    }
    ndk_context::initialize_android_context(vm.cast(), ctx.cast());
}

/// 是否已登记（声音输出前检查，免得 cpal 在线程里 panic）。
pub fn ready() -> bool {
    DONE.load(Ordering::SeqCst)
}
