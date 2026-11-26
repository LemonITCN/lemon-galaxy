//! 连本机控制接口（命名管道）。后台线程负责连接与读，断了每 2 秒重连；另一个线程负责写。
//! 回复与事件交回界面线程处理（`win::ui_post`）；请求的回调都在界面线程上调用。

use crate::control::*;
use crate::pipe::{OpenError, Pipe};
use crate::settings::log_error;
use crate::tr;
use crate::win::{defer, ui_post};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// 给界面的通知
pub enum Signal {
    Connected,
    Disconnected,
    Event(Event),
}

enum Msg {
    Connected(mpsc::Sender<Vec<u8>>, String),
    Line(ServerLine),
    Closed,
}

#[derive(Clone)]
pub struct Client(Rc<Inner>);

pub struct Inner {
    tx: RefCell<Option<mpsc::Sender<Vec<u8>>>>,
    pending: RefCell<HashMap<u64, Box<dyn FnOnce(Response)>>>,
    next: Cell<u64>,
    pipe: RefCell<Option<String>>,
    listeners: RefCell<Vec<Rc<dyn Fn(&Signal)>>>,
}

thread_local! {
    static CURRENT: RefCell<Weak<Inner>> = RefCell::new(Weak::new());
}

fn on_msg(m: Msg) {
    let Some(inner) = CURRENT.with(|c| c.borrow().upgrade()) else { return };
    Client(inner).handle(m);
}

pub fn gone() -> Response {
    Response::err(ErrorCode::Failed, tr!("和被控服务的连接断开了。", "Lost connection to the LemonGalaxy service."))
}

impl Client {
    pub fn start() -> Self {
        let c = Client(Rc::new(Inner { tx: RefCell::new(None), pending: RefCell::new(HashMap::new()), next: Cell::new(1), pipe: RefCell::new(None), listeners: RefCell::new(vec![]) }));
        CURRENT.with(|cur| *cur.borrow_mut() = Rc::downgrade(&c.0));
        if let Err(e) = std::thread::Builder::new().name("control".into()).spawn(connector) {
            log_error(&format!("起不了控制接口线程：{e}"));
        }
        c
    }

    pub fn subscribe(&self, f: impl Fn(&Signal) + 'static) {
        self.0.listeners.borrow_mut().push(Rc::new(f));
    }

    /// 连着的管道名（完整的 \\.\pipe\…）
    pub fn pipe_name(&self) -> Option<String> {
        self.0.pipe.borrow().clone()
    }

    fn emit(&self, s: Signal) {
        let ls: Vec<_> = self.0.listeners.borrow().clone();
        for l in ls {
            l(&s);
        }
    }

    fn handle(&self, m: Msg) {
        match m {
            Msg::Connected(tx, name) => {
                *self.0.tx.borrow_mut() = Some(tx);
                *self.0.pipe.borrow_mut() = Some(name);
                self.emit(Signal::Connected);
            }
            Msg::Line(ServerLine::Response(r)) => {
                let cb = self.0.pending.borrow_mut().remove(&r.id);
                if let Some(cb) = cb {
                    cb(r.resp);
                }
            }
            Msg::Line(ServerLine::Event(e)) => self.emit(Signal::Event(e.event)),
            Msg::Closed => {
                *self.0.tx.borrow_mut() = None;
                // 没等到回复的请求一律失败
                let pending: Vec<_> = self.0.pending.borrow_mut().drain().map(|(_, cb)| cb).collect();
                for cb in pending {
                    cb(gone());
                }
                self.emit(Signal::Disconnected);
            }
        }
    }

    /// 发请求，回复到了在界面线程上调 `cb`。没连上服务时（稍后）回错误
    pub fn call(&self, req: Request, cb: impl FnOnce(Response) + 'static) {
        let tx = self.0.tx.borrow().clone();
        let Some(tx) = tx else {
            defer(move || cb(gone()));
            return;
        };
        let id = self.0.next.get();
        self.0.next.set(id + 1);
        self.0.pending.borrow_mut().insert(id, Box::new(cb));
        if tx.send(encode(&RequestLine { id, req }).into_bytes()).is_err() {
            if let Some(cb) = self.0.pending.borrow_mut().remove(&id) {
                defer(move || cb(gone()));
            }
        }
    }

    /// 不关心回复
    pub fn send(&self, req: Request) {
        self.call(req, |_| {});
    }
}

fn connector() {
    let mut last_problem = String::new();
    loop {
        for (name, system) in crate::pipe::candidates() {
            let pipe = match Pipe::open(&name, system, 400) {
                Ok(p) => Arc::new(p),
                Err(OpenError::Absent) => continue,
                Err(OpenError::Suspicious(m)) => {
                    if m != last_problem {
                        log_error(&m);
                        last_problem = m;
                    }
                    continue;
                }
            };
            serve(pipe, name);
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn serve(pipe: Arc<Pipe>, name: String) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let w = pipe.clone();
    let writer = std::thread::Builder::new().name("control-write".into()).spawn(move || {
        for data in rx {
            if w.write_all(&data, None).is_err() {
                break;
            }
        }
    });
    if writer.is_err() {
        return;
    }
    ui_post(move || on_msg(Msg::Connected(tx, name)));
    let mut buf = vec![0u8; 64 * 1024];
    let mut acc: Vec<u8> = vec![];
    loop {
        let n = match pipe.read(&mut buf, None) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        acc.extend_from_slice(&buf[..n]);
        while let Some(p) = acc.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = acc.drain(..=p).collect();
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            match serde_json::from_str::<ServerLine>(text) {
                Ok(l) => ui_post(move || on_msg(Msg::Line(l))),
                Err(e) => log_error(&format!("控制接口：看不懂的一行（{e}）：{}", text.chars().take(300).collect::<String>())),
            }
        }
        if acc.len() > MAX_LINE {
            break;
        }
    }
    ui_post(|| on_msg(Msg::Closed));
}
