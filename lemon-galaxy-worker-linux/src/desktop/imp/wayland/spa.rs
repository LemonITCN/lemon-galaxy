//! 最小的 SPA POD 构造与解析（PipeWire 的参数格式，见 spa/pod/pod.h）。
//! 头文件里的构造器都是内联函数、库里没有导出，所以按格式自己拼：每个 POD 是 (u32 体长, u32 类型, 体)，体补齐到 8 字节。
//! 常量取自 PipeWire 1.0 的 spa 头文件（这些是 ABI，不会变）。

pub const TYPE_ID: u32 = 3;
pub const TYPE_INT: u32 = 4;
pub const TYPE_RECTANGLE: u32 = 10;
pub const TYPE_FRACTION: u32 = 11;
pub const TYPE_OBJECT: u32 = 15;
pub const TYPE_CHOICE: u32 = 19;

pub const OBJECT_FORMAT: u32 = 0x40003;
pub const OBJECT_PARAM_BUFFERS: u32 = 0x40004;
pub const OBJECT_PARAM_META: u32 = 0x40005;

pub const PARAM_ENUM_FORMAT: u32 = 3;
pub const PARAM_FORMAT: u32 = 4;
pub const PARAM_BUFFERS: u32 = 5;
pub const PARAM_META: u32 = 6;

pub const FORMAT_MEDIA_TYPE: u32 = 1;
pub const FORMAT_MEDIA_SUBTYPE: u32 = 2;
pub const FORMAT_VIDEO_FORMAT: u32 = 0x20001;
pub const FORMAT_VIDEO_MODIFIER: u32 = 0x20002;
pub const FORMAT_VIDEO_SIZE: u32 = 0x20003;
pub const FORMAT_VIDEO_FRAMERATE: u32 = 0x20004;
pub const FORMAT_VIDEO_MAX_FRAMERATE: u32 = 0x20005;

pub const MEDIA_TYPE_VIDEO: u32 = 2;
pub const MEDIA_SUBTYPE_RAW: u32 = 1;

pub const VIDEO_FORMAT_RGBX: u32 = 7;
pub const VIDEO_FORMAT_BGRX: u32 = 8;
pub const VIDEO_FORMAT_XRGB: u32 = 9;
pub const VIDEO_FORMAT_XBGR: u32 = 10;
pub const VIDEO_FORMAT_RGBA: u32 = 11;
pub const VIDEO_FORMAT_BGRA: u32 = 12;
pub const VIDEO_FORMAT_ARGB: u32 = 13;
pub const VIDEO_FORMAT_ABGR: u32 = 14;

pub const BUFFERS_DATA_TYPE: u32 = 6;
pub const META_TYPE: u32 = 1;
pub const META_SIZE: u32 = 2;

pub const DATA_MEM_PTR: u32 = 1;
pub const DATA_MEM_FD: u32 = 2;

pub const META_CURSOR: u32 = 5;

pub const CHOICE_RANGE: u32 = 1;
pub const CHOICE_ENUM: u32 = 3;

/// 光标元数据大小：spa_meta_cursor(28) + spa_meta_bitmap(20) + 宽×高×4
pub const fn cursor_meta_size(w: u32, h: u32) -> u32 {
    28 + 20 + w * h * 4
}

#[derive(Default)]
pub struct Builder {
    buf: Vec<u8>,
}

fn pad8(v: &mut Vec<u8>) {
    while v.len() % 8 != 0 {
        v.push(0);
    }
}

impl Builder {
    fn u32(&mut self, x: u32) {
        self.buf.extend_from_slice(&x.to_le_bytes());
    }

    fn header(&mut self, size: u32, ty: u32) {
        self.u32(size);
        self.u32(ty);
    }

    pub fn id(&mut self, v: u32) {
        self.header(4, TYPE_ID);
        self.u32(v);
        pad8(&mut self.buf);
    }

    pub fn int(&mut self, v: i32) {
        self.header(4, TYPE_INT);
        self.u32(v as u32);
        pad8(&mut self.buf);
    }

    pub fn rectangle(&mut self, w: u32, h: u32) {
        self.header(8, TYPE_RECTANGLE);
        self.u32(w);
        self.u32(h);
    }

    pub fn fraction(&mut self, num: u32, den: u32) {
        self.header(8, TYPE_FRACTION);
        self.u32(num);
        self.u32(den);
    }

    /// 选择：`values` 是同一类型（`child_type`，每个 `child_size` 字节）的若干值，原样拼接好的字节
    pub fn choice(&mut self, kind: u32, child_type: u32, child_size: u32, values: &[u8]) {
        self.header(16 + values.len() as u32, TYPE_CHOICE);
        self.u32(kind);
        self.u32(0);
        self.u32(child_size);
        self.u32(child_type);
        self.buf.extend_from_slice(values);
        pad8(&mut self.buf);
    }

    pub fn choice_ids(&mut self, kind: u32, ids: &[u32]) {
        let v: Vec<u8> = ids.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.choice(kind, TYPE_ID, 4, &v);
    }

    pub fn choice_rects(&mut self, kind: u32, rects: &[(u32, u32)]) {
        let v: Vec<u8> = rects.iter().flat_map(|(w, h)| [w.to_le_bytes(), h.to_le_bytes()].concat()).collect();
        self.choice(kind, TYPE_RECTANGLE, 8, &v);
    }

    pub fn choice_fractions(&mut self, kind: u32, f: &[(u32, u32)]) {
        let v: Vec<u8> = f.iter().flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat()).collect();
        self.choice(kind, TYPE_FRACTION, 8, &v);
    }

    /// 一个对象：`props` 里每项写属性值（通过闭包往同一个 Builder 写一个 POD）
    pub fn object(object_type: u32, id: u32, props: &[(u32, &dyn Fn(&mut Builder))]) -> Vec<u8> {
        let mut body = Builder::default();
        body.u32(object_type);
        body.u32(id);
        for (key, write) in props {
            body.u32(*key);
            body.u32(0);
            write(&mut body);
        }
        let mut out = Builder::default();
        out.header(body.buf.len() as u32, TYPE_OBJECT);
        out.buf.extend_from_slice(&body.buf);
        pad8(&mut out.buf);
        out.buf
    }
}

/// 读一个 POD：返回 (类型, 体)
fn read_pod(b: &[u8]) -> Option<(u32, &[u8], usize)> {
    if b.len() < 8 {
        return None;
    }
    let size = u32::from_le_bytes(b[0..4].try_into().ok()?) as usize;
    let ty = u32::from_le_bytes(b[4..8].try_into().ok()?);
    let body = b.get(8..8 + size)?;
    let total = (8 + size).div_ceil(8) * 8;
    Some((ty, body, total))
}

/// 取对象里某个属性的值（选择类型取第一个值），返回 (类型, 值字节)
pub fn object_prop(pod: &[u8], key: u32) -> Option<(u32, Vec<u8>)> {
    let (ty, body, _) = read_pod(pod)?;
    if ty != TYPE_OBJECT || body.len() < 8 {
        return None;
    }
    let mut off = 8;
    while off + 16 <= body.len() {
        let k = u32::from_le_bytes(body[off..off + 4].try_into().ok()?);
        let (vty, vbody, vlen) = read_pod(&body[off + 8..])?;
        if k == key {
            if vty == TYPE_CHOICE && vbody.len() >= 16 {
                let csize = u32::from_le_bytes(vbody[8..12].try_into().ok()?) as usize;
                let ctype = u32::from_le_bytes(vbody[12..16].try_into().ok()?);
                return Some((ctype, vbody.get(16..16 + csize)?.to_vec()));
            }
            return Some((vty, vbody.to_vec()));
        }
        off += 8 + vlen;
    }
    None
}

pub fn prop_u32(pod: &[u8], key: u32) -> Option<u32> {
    let (_, v) = object_prop(pod, key)?;
    Some(u32::from_le_bytes(v.get(0..4)?.try_into().ok()?))
}

pub fn prop_rect(pod: &[u8], key: u32) -> Option<(u32, u32)> {
    let (ty, v) = object_prop(pod, key)?;
    if ty != TYPE_RECTANGLE {
        return None;
    }
    Some((u32::from_le_bytes(v.get(0..4)?.try_into().ok()?), u32::from_le_bytes(v.get(4..8)?.try_into().ok()?)))
}

/// 屏幕采集要的格式：BGRx 等 32 位格式、任意尺寸、可变帧率（最大 `fps`）。不带 modifier：服务端就给共享内存而不是 dmabuf。
pub fn enum_format(fps: u32) -> Vec<u8> {
    Builder::object(
        OBJECT_FORMAT,
        PARAM_ENUM_FORMAT,
        &[
            (FORMAT_MEDIA_TYPE, &|b: &mut Builder| b.id(MEDIA_TYPE_VIDEO)),
            (FORMAT_MEDIA_SUBTYPE, &|b: &mut Builder| b.id(MEDIA_SUBTYPE_RAW)),
            (FORMAT_VIDEO_FORMAT, &|b: &mut Builder| b.choice_ids(CHOICE_ENUM, &[VIDEO_FORMAT_BGRX, VIDEO_FORMAT_BGRX, VIDEO_FORMAT_BGRA, VIDEO_FORMAT_RGBX, VIDEO_FORMAT_RGBA, VIDEO_FORMAT_XRGB, VIDEO_FORMAT_XBGR])),
            (FORMAT_VIDEO_SIZE, &|b: &mut Builder| b.choice_rects(CHOICE_RANGE, &[(1920, 1080), (1, 1), (16384, 16384)])),
            (FORMAT_VIDEO_FRAMERATE, &|b: &mut Builder| b.fraction(0, 1)),
            (FORMAT_VIDEO_MAX_FRAMERATE, &move |b: &mut Builder| b.choice_fractions(CHOICE_RANGE, &[(fps, 1), (1, 1), (fps, 1)])),
        ],
    )
}

/// 缓冲：只要共享内存（MemFd / MemPtr）
pub fn buffers() -> Vec<u8> {
    Builder::object(OBJECT_PARAM_BUFFERS, PARAM_BUFFERS, &[(BUFFERS_DATA_TYPE, &|b: &mut Builder| b.int(((1 << DATA_MEM_PTR) | (1 << DATA_MEM_FD)) as i32))])
}

/// 元数据：光标（位置 + 最大 256×256 的图像）
pub fn meta_cursor() -> Vec<u8> {
    Builder::object(OBJECT_PARAM_META, PARAM_META, &[(META_TYPE, &|b: &mut Builder| b.id(META_CURSOR)), (META_SIZE, &|b: &mut Builder| b.int(cursor_meta_size(256, 256) as i32))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_parse_format() {
        let p = enum_format(30);
        assert_eq!(p.len() % 8, 0);
        assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), TYPE_OBJECT);
        assert_eq!(u32::from_le_bytes(p[0..4].try_into().unwrap()) as usize + 8, p.len());
        assert_eq!(prop_u32(&p, FORMAT_MEDIA_TYPE), Some(MEDIA_TYPE_VIDEO));
        assert_eq!(prop_u32(&p, FORMAT_VIDEO_FORMAT), Some(VIDEO_FORMAT_BGRX));
        assert_eq!(prop_rect(&p, FORMAT_VIDEO_SIZE), Some((1920, 1080)));
        // 固定值（协商后的格式就是这样）
        let f = Builder::object(OBJECT_FORMAT, PARAM_FORMAT, &[(FORMAT_VIDEO_FORMAT, &|b: &mut Builder| b.id(VIDEO_FORMAT_RGBX)), (FORMAT_VIDEO_SIZE, &|b: &mut Builder| b.rectangle(2560, 1440))]);
        assert_eq!(prop_u32(&f, FORMAT_VIDEO_FORMAT), Some(VIDEO_FORMAT_RGBX));
        assert_eq!(prop_rect(&f, FORMAT_VIDEO_SIZE), Some((2560, 1440)));
        assert_eq!(prop_u32(&f, FORMAT_VIDEO_MODIFIER), None);
    }
}
