//! 复制粘贴文件，Windows 粘贴方（docs/protocol.md 8.4.1）：主控端复制的文件清单放进系统剪贴板，
//! 用的是 OLE 数据对象（FileGroupDescriptorW + FileContents）。资源管理器按 Ctrl+V 或右键粘贴时按项要 IStream，
//! 读的时候才经服务向主控端取数据，显示的是资源管理器自己的「正在复制」进度框（FD_PROGRESSUI），可取消。
//!
//! 跑在剪贴板助手进程（`__lg-clip-helper`）里：服务以控制台会话里登录用户的身份启动它，经标准输入输出通信。
//! 不放在画面代理里，是因为代理以 SYSTEM 身份运行：COM 默认只让同一账号调用，资源管理器（登录用户、中完整性级别）
//! 调不进来；放开进程的 COM 安全会影响代理里别的功能，OLE 剪贴板跨完整性级别（界面特权隔离）也没有把握。
//! 助手与资源管理器同一账号、同一完整性级别，用 COM 默认设置即可，做法与系统自带的 rdpclip.exe 相同。
//! 数据也就不经过代理的管道，不会挤住键鼠事件。
//!
//! 线程：
//! - 剪贴板线程（单线程套间 + 消息循环）：OleSetClipboard，处理资源管理器对数据对象的调用（都很快，只给内存里的东西）；
//! - IStream 放在多线程套间里（在 MTA 线程上封送后再交出去），资源管理器的 Read 落在 COM 的 RPC 线程上，
//!   等网络数据时不挡剪贴板线程；
//! - 数据按需拉取（一次最多 256 KB，一条流同时只有一个请求），不会在管道里堆数据。

use super::ipc::{read_msg, write_msg, FromAgent, ToAgent};
use crate::clipdesc;
use galaxy_proto::peer::ClipOffer;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use windows::core::{IUnknown, IUnknown_Vtbl, Interface, BOOL, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GlobalFree, DATA_S_SAMEFORMATETC, DV_E_DVASPECT, DV_E_FORMATETC, DV_E_LINDEX, DV_E_TYMED, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, FILETIME, HANDLE, HGLOBAL, OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION, STG_E_READFAULT, WAIT_OBJECT_0,
};
use windows::Win32::System::Com::Marshal::CoMarshalInterThreadInterfaceInStream;
use windows::Win32::System::Com::StructuredStorage::CoGetInterfaceAndReleaseStream;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IDataObject, IDataObject_Vtbl, IEnumFORMATETC, IEnumFORMATETC_Vtbl, ISequentialStream, ISequentialStream_Vtbl, IStream, IStream_Vtbl, COINIT_MULTITHREADED, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STATFLAG_NONAME, STATSTG, STGMEDIUM, STGMEDIUM_0, STGM_READ, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END,
    STREAM_SEEK_SET, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard, OleUninitialize};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, TranslateMessage, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, WM_QUIT};

/// 一次向服务要多少（与 QUIC 那头的块一样大）
const CHUNK: usize = 256 * 1024;
/// 打开一项最多等多久（服务那头等主控端应答最多 20 秒）
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// 读一块最多等多久（服务那头 60 秒没有新数据就报错）
const READ_TIMEOUT: Duration = Duration::from_secs(75);
// 描述符的字节布局（clipdesc.rs）与 Windows 头文件一致
const _: () = assert!(std::mem::size_of::<windows::Win32::UI::Shell::FILEDESCRIPTORW>() == clipdesc::DESCRIPTOR_SIZE);

/// 我们自己放上去的清单的标记格式（内容是清单 id）：ClipboardGet 见到它就不把剪贴板当成「本机复制了文件」
const PRIVATE_FORMAT: &str = "LemonGalaxy Clip Offer";

// ---------------- 经服务取数据 ----------------

/// 服务发来的应答
enum Reply {
    /// 打开的结果：文件现在的大小
    Opened(Result<u64, String>),
    /// 一块数据；空表示读到末尾
    Data(Result<Vec<u8>, String>),
}

struct Remote {
    out: SyncSender<FromAgent>,
    waiters: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    next: AtomicU64,
}

static REMOTE: OnceLock<Remote> = OnceLock::new();

/// 助手启动时调用：经 `out` 向服务发请求
fn init(out: SyncSender<FromAgent>) {
    let _ = REMOTE.set(Remote { out, waiters: Mutex::new(HashMap::new()), next: AtomicU64::new(1) });
}

/// 服务的应答到了（助手主循环里调用）
fn deliver(stream: u64, r: Reply) {
    let tx = REMOTE.get().and_then(|x| x.waiters.lock().get(&stream).cloned());
    if let Some(tx) = tx {
        let _ = tx.send(r);
    }
}

fn log(text: String) {
    if let Some(r) = REMOTE.get() {
        let _ = r.out.try_send(FromAgent::Log { error: true, text });
    }
}

/// 服务那头一条打开着的读取（对应主控端的一条反向流）。丢掉即关闭。
struct Conn {
    id: u64,
    rx: mpsc::Receiver<Reply>,
}

impl Conn {
    /// 打开清单第 `index` 项，从 `offset` 读起
    fn open(offer: u64, index: u32, offset: u64) -> Result<Conn, String> {
        let r = REMOTE.get().ok_or("剪贴板未初始化")?;
        let id = r.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        r.waiters.lock().insert(id, tx);
        let c = Conn { id, rx };
        r.out.send(FromAgent::ClipOpen { stream: id, offer, index, offset }).map_err(|_| "被控服务已断开")?;
        match c.rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Reply::Opened(Ok(_))) => Ok(c),
            Ok(Reply::Opened(Err(e))) => Err(e),
            Ok(Reply::Data(_)) => Err("应答顺序不对".into()),
            Err(_) => Err("对方没有应答".into()),
        }
    }

    /// 读一块（最多 `max` 字节）；空表示到了末尾
    fn read(&self, max: usize) -> Result<Vec<u8>, String> {
        let r = REMOTE.get().ok_or("剪贴板未初始化")?;
        r.out.send(FromAgent::ClipRead { stream: self.id, max: max as u32 }).map_err(|_| "被控服务已断开")?;
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Reply::Data(d)) => d,
            Ok(Reply::Opened(_)) => Err("应答顺序不对".into()),
            Err(_) => Err("传输超时".into()),
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Some(r) = REMOTE.get() {
            r.waiters.lock().remove(&self.id);
            let _ = r.out.send(FromAgent::ClipClose { stream: self.id });
        }
    }
}

// ---------------- 剪贴板格式 ----------------

#[derive(Clone, Copy)]
struct Formats {
    descriptor: u16,
    contents: u16,
    effect: u16,
    private: u16,
}

fn register(name: &str) -> u16 {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) as u16 }
}

fn formats() -> Formats {
    static F: OnceLock<Formats> = OnceLock::new();
    *F.get_or_init(|| Formats { descriptor: register("FileGroupDescriptorW"), contents: register("FileContents"), effect: register("Preferred DropEffect"), private: register(PRIVATE_FORMAT) })
}

/// 我们放上去的清单的标记格式编号（ClipboardGet 判断用）
pub fn private_format() -> u32 {
    formats().private as u32
}

// ---------------- 手写的 COM 对象 ----------------
//
// 不用 windows 库的 #[implement]：它生成的对象总会带上自由线程封送器（弱引用那部分也会），而那个函数
// windows 库从 combase.dll 导入，Windows 7 及更老的系统没有这个 DLL，兼容版程序会起不来。
// 也正好要标准封送：IStream 才会真的挂在多线程套间里（见 into_mta）。

/// 一个 COM 对象：虚表指针在最前面，后面是引用计数和内容。
#[repr(C)]
struct Com<T: Class> {
    vtbl: &'static T::Vtbl,
    refs: AtomicU32,
    inner: T,
}

trait Class: Sized + Send + Sync + 'static {
    type Vtbl: 'static;
    const VTBL: &'static Self::Vtbl;
    /// 除 IUnknown 外还认哪些接口
    const IIDS: &'static [GUID];
}

impl<T: Class> Com<T> {
    /// 新对象（引用计数 1），返回接口指针
    fn create<I: Interface>(inner: T) -> I {
        let p = Box::into_raw(Box::new(Com { vtbl: T::VTBL, refs: AtomicU32::new(1), inner }));
        unsafe { I::from_raw(p as *mut c_void) }
    }

    /// 接口指针 → 内容
    unsafe fn get<'a>(this: *mut c_void) -> &'a T {
        unsafe { &(*(this as *const Com<T>)).inner }
    }
}

const fn unknown<T: Class>() -> IUnknown_Vtbl {
    IUnknown_Vtbl { QueryInterface: query_interface::<T>, AddRef: add_ref::<T>, Release: release::<T> }
}

unsafe extern "system" fn query_interface<T: Class>(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        if out.is_null() {
            return E_POINTER;
        }
        if !iid.is_null() && (*iid == IUnknown::IID || T::IIDS.contains(&*iid)) {
            add_ref::<T>(this);
            *out = this;
            return S_OK;
        }
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref<T: Class>(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Com<T>)).refs.fetch_add(1, Ordering::Relaxed) + 1 }
}

unsafe extern "system" fn release<T: Class>(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Com<T>)).refs.fetch_sub(1, Ordering::Release) - 1;
        if n == 0 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this as *mut Com<T>));
        }
        n
    }
}

/// 给 *out 写一个接口指针（out 为空时什么也不做）
unsafe fn put_out(out: *mut *mut c_void, p: *mut c_void) {
    if !out.is_null() {
        unsafe { *out = p };
    }
}

// ---------------- 数据对象 ----------------

struct Item {
    is_dir: bool,
    size: u64,
    mtime: i64,
    name: String,
}

struct OfferData {
    id: u64,
    descriptor: Vec<u8>,
    items: Vec<Item>,
}

fn hglobal(data: &[u8]) -> Result<HGLOBAL, HRESULT> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)).map_err(|e| e.code())?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(E_OUTOFMEMORY);
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

fn hglobal_medium(data: &[u8]) -> Result<STGMEDIUM, HRESULT> {
    Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: hglobal(data)? }, pUnkForRelease: ManuallyDrop::new(None) })
}

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

/// 把 IStream 挂到多线程套间：在 MTA 线程上封送，再在本线程（剪贴板的单线程套间）解封得到 COM 代理（proxy）。
/// 交给资源管理器的是这个 proxy，它的调用直接进多线程套间（COM 的 RPC 线程），Read 阻塞也不挡剪贴板线程。
fn into_mta(stream: IStream) -> Result<IStream, HRESULT> {
    let raw = SendPtr(stream.into_raw());
    let packet = std::thread::spawn(move || unsafe {
        // 整个搬进来（只拿字段的话闭包抓的是裸指针，不能跨线程）
        let raw = raw;
        // 主线程一直在多线程套间里（helper_main），这个线程退出后存根仍在
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let s = IStream::from_raw(raw.0);
        let r = CoMarshalInterThreadInterfaceInStream(&IStream::IID, &s).map(|p| SendPtr(p.into_raw())).map_err(|e| e.code());
        drop(s);
        CoUninitialize();
        r
    })
    .join()
    .map_err(|_| E_UNEXPECTED)??;
    unsafe {
        let p = IStream::from_raw(packet.0);
        let r = CoGetInterfaceAndReleaseStream::<_, IStream>(&p).map_err(|e| e.code());
        // CoGetInterfaceAndReleaseStream 已经释放了它
        std::mem::forget(p);
        r
    }
}

struct DataObject {
    offer: Arc<OfferData>,
}

impl Class for DataObject {
    type Vtbl = IDataObject_Vtbl;
    const VTBL: &'static IDataObject_Vtbl = &IDataObject_Vtbl {
        base__: unknown::<DataObject>(),
        GetData: DataObject::get_data,
        GetDataHere: DataObject::get_data_here,
        QueryGetData: DataObject::query_get_data,
        GetCanonicalFormatEtc: DataObject::canonical,
        SetData: DataObject::set_data,
        EnumFormatEtc: DataObject::enum_format_etc,
        DAdvise: DataObject::d_advise,
        DUnadvise: DataObject::d_unadvise,
        EnumDAdvise: DataObject::enum_d_advise,
    };
    const IIDS: &'static [GUID] = &[IDataObject::IID];
}

impl DataObject {
    /// 有没有这种格式；`lindex` 为真时连 FileContents 的下标一起查（QueryGetData 不查：常有人用 -1 问有没有这种格式）
    fn check(&self, f: &FORMATETC, lindex: bool) -> HRESULT {
        let fm = formats();
        if f.dwAspect != DVASPECT_CONTENT.0 && f.dwAspect != 0 {
            return DV_E_DVASPECT;
        }
        let want = if f.cfFormat == fm.contents {
            if lindex && self.item_index(f.lindex).is_none() {
                return DV_E_LINDEX;
            }
            TYMED_ISTREAM.0 as u32
        } else if [fm.descriptor, fm.effect, fm.private].contains(&f.cfFormat) {
            TYMED_HGLOBAL.0 as u32
        } else {
            return DV_E_FORMATETC;
        };
        if f.tymed & want == 0 {
            return DV_E_TYMED;
        }
        S_OK
    }

    /// FileContents 的 lindex → 清单下标。-1 不认：剪贴板管理工具经 Win32 剪贴板要 FileContents 时用的是 -1，
    /// 认了的话 OLE 会把整个文件读进内存，等于复制时就开始传
    fn item_index(&self, lindex: i32) -> Option<usize> {
        let i = usize::try_from(lindex).ok()?;
        self.offer.items.get(i).filter(|x| !x.is_dir).map(|_| i)
    }

    fn medium(&self, f: &FORMATETC) -> Result<STGMEDIUM, HRESULT> {
        let hr = self.check(f, true);
        if hr != S_OK {
            return Err(hr);
        }
        let fm = formats();
        if f.cfFormat == fm.descriptor {
            hglobal_medium(&self.offer.descriptor)
        } else if f.cfFormat == fm.effect {
            hglobal_medium(&1u32.to_le_bytes()) // DROPEFFECT_COPY：复制，不是剪切
        } else if f.cfFormat == fm.private {
            hglobal_medium(&self.offer.id.to_le_bytes())
        } else {
            let i = self.item_index(f.lindex).ok_or(DV_E_LINDEX)?;
            let it = &self.offer.items[i];
            let stream: IStream = Com::create(ClipStream { offer: self.offer.id, index: i as u32, size: it.size, mtime: it.mtime, name: it.name.clone(), st: Mutex::new(StreamState::default()) });
            let stream = into_mta(stream)?;
            Ok(STGMEDIUM { tymed: TYMED_ISTREAM.0 as u32, u: STGMEDIUM_0 { pstm: ManuallyDrop::new(Some(stream)) }, pUnkForRelease: ManuallyDrop::new(None) })
        }
    }

    unsafe extern "system" fn get_data(this: *mut c_void, f: *const FORMATETC, out: *mut STGMEDIUM) -> HRESULT {
        unsafe {
            let (Some(f), false) = (f.as_ref(), out.is_null()) else { return E_INVALIDARG };
            match Com::<Self>::get(this).medium(f) {
                Ok(m) => {
                    std::ptr::write(out, m);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
    }

    unsafe extern "system" fn get_data_here(_this: *mut c_void, _f: *const FORMATETC, _m: *mut STGMEDIUM) -> HRESULT {
        E_NOTIMPL
    }

    unsafe extern "system" fn query_get_data(this: *mut c_void, f: *const FORMATETC) -> HRESULT {
        unsafe {
            match f.as_ref() {
                Some(f) => Com::<Self>::get(this).check(f, false),
                None => E_INVALIDARG,
            }
        }
    }

    unsafe extern "system" fn canonical(_this: *mut c_void, _f: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        unsafe {
            if let Some(o) = out.as_mut() {
                o.ptd = null_mut();
            }
        }
        DATA_S_SAMEFORMATETC
    }

    unsafe extern "system" fn set_data(_this: *mut c_void, _f: *const FORMATETC, _m: *const STGMEDIUM, _release: BOOL) -> HRESULT {
        E_NOTIMPL
    }

    unsafe extern "system" fn enum_format_etc(_this: *mut c_void, dir: u32, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            if out.is_null() {
                return E_POINTER;
            }
            if dir != DATADIR_GET.0 as u32 {
                *out = null_mut();
                return E_NOTIMPL;
            }
            let fm = formats();
            let list = vec![(fm.descriptor, TYMED_HGLOBAL.0 as u32), (fm.contents, TYMED_ISTREAM.0 as u32), (fm.effect, TYMED_HGLOBAL.0 as u32), (fm.private, TYMED_HGLOBAL.0 as u32)];
            *out = Com::create::<IEnumFORMATETC>(FormatEnum { list: Arc::new(list), pos: AtomicUsize::new(0) }).into_raw();
            S_OK
        }
    }

    unsafe extern "system" fn d_advise(_this: *mut c_void, _f: *const FORMATETC, _advf: u32, _sink: *mut c_void, _conn: *mut u32) -> HRESULT {
        OLE_E_ADVISENOTSUPPORTED
    }

    unsafe extern "system" fn d_unadvise(_this: *mut c_void, _conn: u32) -> HRESULT {
        OLE_E_ADVISENOTSUPPORTED
    }

    unsafe extern "system" fn enum_d_advise(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
        unsafe { put_out(out, null_mut()) };
        OLE_E_ADVISENOTSUPPORTED
    }
}

struct FormatEnum {
    list: Arc<Vec<(u16, u32)>>,
    pos: AtomicUsize,
}

impl Class for FormatEnum {
    type Vtbl = IEnumFORMATETC_Vtbl;
    const VTBL: &'static IEnumFORMATETC_Vtbl = &IEnumFORMATETC_Vtbl { base__: unknown::<FormatEnum>(), Next: FormatEnum::next, Skip: FormatEnum::skip, Reset: FormatEnum::reset, Clone: FormatEnum::clone };
    const IIDS: &'static [GUID] = &[IEnumFORMATETC::IID];
}

impl FormatEnum {
    unsafe extern "system" fn next(this: *mut c_void, celt: u32, rgelt: *mut FORMATETC, fetched: *mut u32) -> HRESULT {
        unsafe {
            let me = Com::<Self>::get(this);
            if rgelt.is_null() {
                return E_POINTER;
            }
            let mut n = 0usize;
            while n < celt as usize {
                let i = me.pos.fetch_add(1, Ordering::Relaxed);
                let Some(&(cf, tymed)) = me.list.get(i) else {
                    me.pos.store(me.list.len(), Ordering::Relaxed);
                    break;
                };
                std::ptr::write(rgelt.add(n), FORMATETC { cfFormat: cf, ptd: null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed });
                n += 1;
            }
            if let Some(p) = fetched.as_mut() {
                *p = n as u32;
            }
            if n == celt as usize {
                S_OK
            } else {
                S_FALSE
            }
        }
    }

    unsafe extern "system" fn skip(this: *mut c_void, celt: u32) -> HRESULT {
        let me = unsafe { Com::<Self>::get(this) };
        let p = me.pos.fetch_add(celt as usize, Ordering::Relaxed) + celt as usize;
        if p <= me.list.len() {
            S_OK
        } else {
            me.pos.store(me.list.len(), Ordering::Relaxed);
            S_FALSE
        }
    }

    unsafe extern "system" fn reset(this: *mut c_void) -> HRESULT {
        unsafe { Com::<Self>::get(this) }.pos.store(0, Ordering::Relaxed);
        S_OK
    }

    unsafe extern "system" fn clone(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            if out.is_null() {
                return E_POINTER;
            }
            let me = Com::<Self>::get(this);
            *out = Com::create::<IEnumFORMATETC>(FormatEnum { list: me.list.clone(), pos: AtomicUsize::new(me.pos.load(Ordering::Relaxed)) }).into_raw();
            S_OK
        }
    }
}

// ---------------- 按项的 IStream ----------------

#[derive(Default)]
struct StreamState {
    pos: u64,
    /// 打开着的读取（从 pos 处接着读）；Seek 到别处时关掉，下次 Read 在新位置重开
    conn: Option<Conn>,
    /// 在这个位置读到过末尾
    eof_at: Option<u64>,
}

struct ClipStream {
    offer: u64,
    index: u32,
    size: u64,
    mtime: i64,
    name: String,
    st: Mutex<StreamState>,
}

impl Class for ClipStream {
    type Vtbl = IStream_Vtbl;
    const VTBL: &'static IStream_Vtbl = &IStream_Vtbl {
        base__: ISequentialStream_Vtbl { base__: unknown::<ClipStream>(), Read: ClipStream::read, Write: ClipStream::write },
        Seek: ClipStream::seek,
        SetSize: ClipStream::set_size,
        CopyTo: ClipStream::copy_to,
        Commit: ClipStream::commit,
        Revert: ClipStream::revert,
        LockRegion: ClipStream::lock_region,
        UnlockRegion: ClipStream::lock_region,
        Stat: ClipStream::stat,
        Clone: ClipStream::clone,
    };
    const IIDS: &'static [GUID] = &[IStream::IID, ISequentialStream::IID];
}

impl ClipStream {
    /// 读满 `buf`（到末尾或出错时少读）。返回读到的字节数；一个字节都没读到就出错时返回错误
    fn fill(&self, buf: &mut [u8]) -> Result<usize, String> {
        let mut st = self.st.lock();
        let mut done = 0;
        while done < buf.len() {
            if st.eof_at == Some(st.pos) {
                break;
            }
            if st.conn.is_none() {
                match Conn::open(self.offer, self.index, st.pos) {
                    Ok(c) => st.conn = Some(c),
                    Err(e) if done == 0 => return Err(e),
                    Err(_) => break,
                }
            }
            let want = (buf.len() - done).min(CHUNK);
            match st.conn.as_ref().map(|c| c.read(want)).unwrap_or_else(|| Err("没有打开".into())) {
                Ok(d) if d.is_empty() => {
                    st.eof_at = Some(st.pos);
                    st.conn = None;
                    break;
                }
                Ok(d) => {
                    let n = d.len().min(want);
                    buf[done..done + n].copy_from_slice(&d[..n]);
                    done += n;
                    st.pos += n as u64;
                }
                Err(e) => {
                    // 下次 Read（资源管理器的「重试」）在当前位置重开
                    st.conn = None;
                    if done == 0 {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        Ok(done)
    }

    unsafe extern "system" fn read(this: *mut c_void, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        unsafe {
            let me = Com::<Self>::get(this);
            if pv.is_null() {
                return E_POINTER;
            }
            let buf = std::slice::from_raw_parts_mut(pv as *mut u8, cb as usize);
            let r = me.fill(buf);
            if let Some(p) = pcbread.as_mut() {
                *p = *r.as_ref().unwrap_or(&0) as u32;
            }
            match r {
                Ok(_) => S_OK,
                Err(e) => {
                    log(format!("粘贴文件「{}」读取失败：{e}", me.name));
                    STG_E_READFAULT
                }
            }
        }
    }

    unsafe extern "system" fn write(_this: *mut c_void, _pv: *const c_void, _cb: u32, written: *mut u32) -> HRESULT {
        unsafe {
            if let Some(p) = written.as_mut() {
                *p = 0;
            }
        }
        STG_E_ACCESSDENIED
    }

    unsafe extern "system" fn seek(this: *mut c_void, dlibmove: i64, origin: STREAM_SEEK, newpos: *mut u64) -> HRESULT {
        unsafe {
            let me = Com::<Self>::get(this);
            let mut st = me.st.lock();
            let base = match origin {
                STREAM_SEEK_SET => 0i128,
                STREAM_SEEK_CUR => st.pos as i128,
                STREAM_SEEK_END => me.size as i128,
                _ => return STG_E_INVALIDFUNCTION,
            };
            let pos = base + dlibmove as i128;
            if pos < 0 || pos > u64::MAX as i128 {
                return STG_E_INVALIDFUNCTION;
            }
            if pos as u64 != st.pos {
                st.conn = None;
                st.pos = pos as u64;
            }
            if let Some(p) = newpos.as_mut() {
                *p = st.pos;
            }
            S_OK
        }
    }

    unsafe extern "system" fn set_size(_this: *mut c_void, _size: u64) -> HRESULT {
        STG_E_ACCESSDENIED
    }

    unsafe extern "system" fn copy_to(this: *mut c_void, pstm: *mut c_void, cb: u64, pcbread: *mut u64, pcbwritten: *mut u64) -> HRESULT {
        unsafe {
            let me = Com::<Self>::get(this);
            let Some(dst) = IStream::from_raw_borrowed(&pstm) else { return E_POINTER };
            let mut buf = vec![0u8; CHUNK];
            let (mut read, mut written) = (0u64, 0u64);
            let mut hr = S_OK;
            while read < cb {
                let want = (cb - read).min(CHUNK as u64) as usize;
                let n = match me.fill(&mut buf[..want]) {
                    Ok(n) => n,
                    Err(e) => {
                        log(format!("粘贴文件「{}」读取失败：{e}", me.name));
                        hr = STG_E_READFAULT;
                        break;
                    }
                };
                if n == 0 {
                    break;
                }
                read += n as u64;
                let mut w = 0u32;
                let r = dst.Write(buf.as_ptr() as _, n as u32, Some(&mut w));
                written += w as u64;
                if r.is_err() {
                    hr = r;
                    break;
                }
                if n < want {
                    break;
                }
            }
            if let Some(p) = pcbread.as_mut() {
                *p = read;
            }
            if let Some(p) = pcbwritten.as_mut() {
                *p = written;
            }
            hr
        }
    }

    unsafe extern "system" fn commit(_this: *mut c_void, _flags: u32) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn revert(_this: *mut c_void) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn lock_region(_this: *mut c_void, _offset: u64, _cb: u64, _kind: u32) -> HRESULT {
        STG_E_INVALIDFUNCTION
    }

    unsafe extern "system" fn stat(this: *mut c_void, out: *mut STATSTG, flag: u32) -> HRESULT {
        unsafe {
            let me = Com::<Self>::get(this);
            if out.is_null() {
                return E_POINTER;
            }
            let mut name = PWSTR::null();
            if flag & STATFLAG_NONAME.0 as u32 == 0 {
                let w: Vec<u16> = me.name.encode_utf16().chain(Some(0)).collect();
                let p = super::compat::co_task_mem_alloc(w.len() * 2) as *mut u16;
                if p.is_null() {
                    return E_OUTOFMEMORY;
                }
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                name = PWSTR(p);
            }
            let t = clipdesc::filetime(me.mtime).unwrap_or(0);
            let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
            std::ptr::write(out, STATSTG { pwcsName: name, r#type: STGTY_STREAM.0 as u32, cbSize: me.size, mtime: ft, ctime: ft, atime: ft, grfMode: STGM_READ, grfLocksSupported: 0, clsid: GUID::zeroed(), grfStateBits: 0, reserved: 0 });
            S_OK
        }
    }

    unsafe extern "system" fn clone(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
        unsafe { put_out(out, null_mut()) };
        E_NOTIMPL
    }
}

// ---------------- 剪贴板线程 ----------------

enum Job {
    Set(Arc<OfferData>, Box<dyn FnOnce(Result<(), String>) + Send>),
    Quit(mpsc::Sender<()>),
}

struct OleThread {
    jobs: mpsc::Sender<Job>,
    wake: HANDLE,
}
unsafe impl Send for OleThread {}

static OLE: Mutex<Option<OleThread>> = Mutex::new(None);

/// 交给剪贴板线程（没有就先起一个）；失败时把任务还回来
fn post(job: Job) -> Result<(), (String, Job)> {
    let mut g = OLE.lock();
    if g.is_none() {
        let wake = match unsafe { CreateEventW(None, false, false, PCWSTR::null()) } {
            Ok(h) => h,
            Err(e) => return Err((e.to_string(), job)),
        };
        let (tx, rx) = mpsc::channel();
        let w = SendPtr(wake.0);
        if let Err(e) = std::thread::Builder::new().name("lg-clipboard-ole".into()).spawn(move || {
            let w = w;
            ole_thread(rx, HANDLE(w.0))
        }) {
            unsafe {
                let _ = CloseHandle(wake);
            }
            return Err((e.to_string(), job));
        }
        *g = Some(OleThread { jobs: tx, wake });
    }
    let t = g.as_ref().unwrap();
    if let Err(e) = t.jobs.send(job) {
        // 线程已经退了（助手正在退出）
        *g = None;
        return Err(("剪贴板线程已退出".into(), e.0));
    }
    unsafe {
        let _ = SetEvent(t.wake);
    }
    Ok(())
}

#[link(name = "ole32")]
extern "system" {
    // windows 库的包装把 S_FALSE（不是）也当成功，这里要原始返回值
    fn OleIsCurrentClipboard(pdataobj: *mut c_void) -> HRESULT;
}

fn ole_thread(rx: mpsc::Receiver<Job>, wake: HANDLE) {
    let init = unsafe { OleInitialize(None) };
    let mut current: Option<IDataObject> = None;
    'outer: loop {
        // 消息与任务都等：COM 调用、剪贴板消息靠消息循环，任务靠事件（线程消息在 COM 模态循环里可能丢）
        let r = unsafe { MsgWaitForMultipleObjectsEx(Some(&[wake]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if r == WAIT_OBJECT_0 {
            while let Ok(j) = rx.try_recv() {
                match j {
                    Job::Set(offer, done) => {
                        let result = match &init {
                            Err(e) => Err(format!("初始化 OLE 失败：{e}")),
                            Ok(()) => {
                                let obj: IDataObject = Com::create(DataObject { offer });
                                match unsafe { OleSetClipboard(&obj) } {
                                    Ok(()) => {
                                        current = Some(obj);
                                        Ok(())
                                    }
                                    Err(e) => Err(format!("放进剪贴板失败：{e}")),
                                }
                            }
                        };
                        done(result);
                    }
                    Job::Quit(done) => {
                        // 助手退出：剪贴板上还是我们的清单就清掉（进程没了它也读不了）
                        if let Some(o) = current.take() {
                            if unsafe { OleIsCurrentClipboard(o.as_raw()) } == S_OK {
                                let _ = unsafe { OleSetClipboard(None::<&IDataObject>) };
                            }
                        }
                        let _ = done.send(());
                        break 'outer;
                    }
                }
            }
        }
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                break 'outer;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // 剪贴板已被别人换掉：不再占着这份清单
        if let Some(o) = &current {
            if unsafe { OleIsCurrentClipboard(o.as_raw()) } != S_OK {
                current = None;
            }
        }
    }
    drop(current);
    if init.is_ok() {
        unsafe { OleUninitialize() };
    }
    unsafe {
        let _ = CloseHandle(wake);
    }
}

/// 把主控端复制的文件清单放进剪贴板；结果经 `done` 回报（在剪贴板线程里调用它）。
fn set_offer(offer: ClipOffer, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let descriptor = match clipdesc::file_group_descriptor(&offer.entries) {
        Ok(d) => d,
        Err(e) => return done(Err(e)),
    };
    let items = offer.entries.iter().map(|e| Item { is_dir: e.is_dir, size: e.size, mtime: e.mtime, name: clipdesc::windows_name(e.path.rsplit('/').next().unwrap_or_default()) }).collect();
    let data = Arc::new(OfferData { id: offer.id, descriptor, items });
    if let Err((e, Job::Set(_, done))) = post(Job::Set(data, Box::new(done))) {
        done(Err(e));
    }
}

/// 退出前调用：剪贴板上还是我们的清单就清掉。最多等 2 秒。
fn shutdown() {
    let (tx, rx) = mpsc::channel();
    let running = OLE.lock().is_some();
    if running && post(Job::Quit(tx)).is_ok() {
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

// ---------------- 助手进程 ----------------

/// 剪贴板助手（`__lg-clip-helper`）：从标准输入读服务的消息（清单、数据），向标准输出写请求。服务关掉管道时
/// 清掉剪贴板上我们的清单（来源已经没了）后退出。
pub fn helper_main() -> i32 {
    // 主线程进多线程套间并一直留着：IStream 的存根挂在这个套间里
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let (out_tx, out_rx) = mpsc::sync_channel::<FromAgent>(64);
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        while let Ok(m) = out_rx.recv() {
            if write_msg(&mut w, &m).is_err() {
                break;
            }
        }
    });
    init(out_tx.clone());
    let stdin = std::io::stdin();
    let mut r = std::io::BufReader::with_capacity(512 * 1024, stdin.lock());
    while let Ok(Some(m)) = read_msg::<_, ToAgent>(&mut r) {
        match m {
            ToAgent::ClipOffer { id, offer } => {
                let out = out_tx.clone();
                set_offer(offer, move |result| {
                    let _ = out.send(FromAgent::ClipOfferSet { id, result });
                });
            }
            ToAgent::ClipOpened { stream, result } => deliver(stream, Reply::Opened(result)),
            ToAgent::ClipData { stream, result } => deliver(stream, Reply::Data(result)),
            _ => {}
        }
    }
    shutdown();
    // 别的线程（RPC 线程上的读取）还拿着输出通道，不等它们
    std::process::exit(0)
}
