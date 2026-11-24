//! 连本机控制接口（docs/linux-desktop.md 第 7 节）。后台线程负责连接与读，断了每 2 秒重连；
//! 回复与事件交回 GTK 主线程处理，写在主线程（请求都很短）。

use crate::control::*;
use crate::tr;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

enum Msg {
    Connected(UnixStream, PathBuf),
    Line(ServerLine),
    Closed,
}

/// 给界面的通知
pub enum Signal {
    Connected,
    Disconnected,
    Event(Event),
}

#[derive(Clone)]
pub struct Client(Rc<Inner>);

struct Inner {
    writer: RefCell<Option<UnixStream>>,
    pending: RefCell<HashMap<u64, async_channel::Sender<Response>>>,
    next: Cell<u64>,
    listeners: RefCell<Vec<Rc<dyn Fn(&Signal)>>>,
    path: RefCell<Option<PathBuf>>,
}

impl Client {
    pub fn start() -> Self {
        let (tx, rx) = async_channel::unbounded();
        std::thread::Builder::new().name("control".into()).spawn(move || connector(tx)).expect("thread");
        let c = Client(Rc::new(Inner { writer: RefCell::new(None), pending: RefCell::new(HashMap::new()), next: Cell::new(1), listeners: RefCell::new(vec![]), path: RefCell::new(None) }));
        let c2 = c.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(m) = rx.recv().await {
                c2.on_msg(m);
            }
        });
        c
    }

    pub fn subscribe(&self, f: impl Fn(&Signal) + 'static) {
        self.0.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn socket_path(&self) -> Option<PathBuf> {
        self.0.path.borrow().clone()
    }

    fn emit(&self, s: Signal) {
        let ls: Vec<_> = self.0.listeners.borrow().clone();
        for l in ls {
            l(&s);
        }
    }

    fn on_msg(&self, m: Msg) {
        match m {
            Msg::Connected(w, p) => {
                *self.0.writer.borrow_mut() = Some(w);
                *self.0.path.borrow_mut() = Some(p);
                self.emit(Signal::Connected);
            }
            Msg::Line(ServerLine::Response(r)) => {
                if let Some(tx) = self.0.pending.borrow_mut().remove(&r.id) {
                    let _ = tx.try_send(r.resp);
                }
            }
            Msg::Line(ServerLine::Event(e)) => self.emit(Signal::Event(e.event)),
            Msg::Closed => {
                *self.0.writer.borrow_mut() = None;
                // 没等到回复的请求一律失败
                let pending: Vec<_> = self.0.pending.borrow_mut().drain().collect();
                for (_, tx) in pending {
                    let _ = tx.try_send(gone());
                }
                self.emit(Signal::Disconnected);
            }
        }
    }

    /// 发请求，等回复。没连上服务时马上返回错误。
    pub async fn call(&self, req: Request) -> Response {
        let id = self.0.next.get();
        self.0.next.set(id + 1);
        let (tx, rx) = async_channel::bounded(1);
        {
            let mut w = self.0.writer.borrow_mut();
            let Some(s) = w.as_mut() else { return gone() };
            if s.write_all(encode(&RequestLine { id, req }).as_bytes()).is_err() {
                return gone();
            }
        }
        self.0.pending.borrow_mut().insert(id, tx);
        rx.recv().await.unwrap_or_else(|_| gone())
    }
}

fn gone() -> Response {
    Response::err(ErrorCode::Failed, tr!("和被控服务的连接断开了。", "Lost connection to the LemonGalaxy service."))
}

fn connector(tx: async_channel::Sender<Msg>) {
    loop {
        for p in socket_candidates() {
            let Ok(s) = UnixStream::connect(&p) else { continue };
            let Ok(w) = s.try_clone() else { continue };
            if tx.send_blocking(Msg::Connected(w, p.clone())).is_err() {
                return;
            }
            let mut r = BufReader::new(s);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                match serde_json::from_str::<ServerLine>(line.trim()) {
                    Ok(l) => {
                        if tx.send_blocking(Msg::Line(l)).is_err() {
                            return;
                        }
                    }
                    Err(e) => eprintln!("control: {e}: {}", line.trim()),
                }
            }
            if tx.send_blocking(Msg::Closed).is_err() {
                return;
            }
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
