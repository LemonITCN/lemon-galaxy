#!/usr/bin/env python3
"""被控端自动更新端到端测试的辅助工具（scripts/e2e-update.sh 调用），只用标准库。

  publish <主机:端口> <管理员> <密码> <两步验证密钥文件> <文件…>
        以管理员登录后台（第一次绑定两步验证，密钥存进那个文件，以后按它算验证码），把文件连同同目录的
        SHA256SUMS.txt 与 SHA256SUMS.txt.sig 上传到「更新」（服务器验发布签名、按文件名分类）
  ctl <system|套接字路径> <status|apply|check>
        经被控服务的本机控制接口发一个请求，打印回复的 JSON（system：Linux 系统服务的套接字 / Windows 系统服务的管道）
  patch <源> <目标> <旧串>=<新串> [...]
        复制程序文件并把字节串原样替换（长度必须相同，每个都至少要替换到一处）
  zip <输出.zip> <目录>
        把目录打成 zip：顶层是同名文件夹、路径用反斜杠，与 PowerShell 的 Compress-Archive 一致
"""
import base64, hashlib, hmac, json, os, socket, ssl, struct, sys, time, urllib.error, urllib.request, uuid, zipfile

# Windows 上的 Python 标准输出默认是系统代码页（GitHub 的 Windows 机器是 cp1252），打印中文会抛 UnicodeEncodeError
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


def totp(secret_b32):
    s = secret_b32.strip().upper()
    key = base64.b32decode(s + "=" * (-len(s) % 8))
    mac = hmac.new(key, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
    o = mac[-1] & 0x0F
    return "%06d" % ((struct.unpack(">I", mac[o:o + 4])[0] & 0x7FFFFFFF) % 1000000)


class Admin:
    def __init__(self, host):
        self.base = f"https://{host}/api/v1/admin"
        self.ctx = ssl.create_default_context()
        self.ctx.check_hostname = False
        self.ctx.verify_mode = ssl.CERT_NONE
        self.cookie = ""
        self.csrf = ""

    def call(self, path, body=None, ctype="application/json"):
        data = json.dumps(body).encode() if ctype == "application/json" and body is not None else body
        r = urllib.request.Request(self.base + path, data=data, method="POST" if data is not None else "GET")
        if data is not None:
            r.add_header("Content-Type", ctype)
        if self.cookie:
            r.add_header("Cookie", self.cookie)
        if self.csrf:
            r.add_header("x-csrf-token", self.csrf)
        try:
            with urllib.request.urlopen(r, context=self.ctx, timeout=600) as resp:
                for k, v in resp.headers.items():
                    if k.lower() == "set-cookie" and v.startswith("lg_admin="):
                        self.cookie = v.split(";")[0]
                text = resp.read().decode()
                return resp.status, (json.loads(text) if text.strip() else {})
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")
            try:
                return e.code, json.loads(text)
            except ValueError:
                return e.code, {"raw": text}

    def login(self, user, password, secret_file):
        secret = open(secret_file).read().strip() if os.path.exists(secret_file) else ""
        body = {"username": user, "password": password}
        if secret:
            body["totp_code"] = totp(secret)
        code, r = self.call("/login", body)
        if code != 200:
            sys.exit(f"管理员登录失败：{code} {r}")
        self.csrf = r.get("csrf_token", "")
        if r.get("stage") == "totp_setup":
            code, r = self.call("/totp/setup", {})
            if code != 200:
                sys.exit(f"绑定两步验证失败：{code} {r}")
            secret = r["secret"]
            open(secret_file, "w").write(secret)
            code, r = self.call("/totp/verify", {"code": totp(secret)})
            if code != 200:
                sys.exit(f"两步验证码不对：{code} {r}")

    def upload(self, path):
        d = os.path.dirname(os.path.abspath(path))
        sums = open(os.path.join(d, "SHA256SUMS.txt"), "rb").read()
        sig = open(os.path.join(d, "SHA256SUMS.txt.sig"), "rb").read()
        b = "----lg" + uuid.uuid4().hex
        parts = []
        for name, filename, content in [("file", os.path.basename(path), open(path, "rb").read()), ("sums", None, sums), ("signature", None, sig)]:
            head = f'--{b}\r\nContent-Disposition: form-data; name="{name}"'
            head += f'; filename="{filename}"\r\nContent-Type: application/octet-stream' if filename else ""
            parts.append(head.encode() + b"\r\n\r\n" + content + b"\r\n")
        body = b"".join(parts) + f"--{b}--\r\n".encode()
        code, r = self.call("/updates/upload", body, f"multipart/form-data; boundary={b}")
        if code != 200:
            sys.exit(f"上传 {os.path.basename(path)} 失败：{code} {r}")
        print(f"已上传 {r.get('file_name')}：{r.get('component')} / {r.get('platform')} / {r.get('arch')} {r.get('version')}")


def publish(host, user, password, secret_file, files):
    a = Admin(host)
    a.login(user, password, secret_file)
    for f in files:
        a.upload(f)


def ctl(where, op):
    if where == "system":
        where = r"\\.\pipe\LemonGalaxyWorker.control" if os.name == "nt" else "/run/lemon-galaxy-worker/control.sock"
    lines = [json.dumps({"id": 1, "req": {"op": "hello", "lang": "en"}}), json.dumps({"id": 2, "req": {"op": {"check": "check_update", "apply": "apply_update"}.get(op, op)}})]
    payload = ("\n".join(lines) + "\n").encode()
    if os.name == "nt":
        for _ in range(50):
            try:
                f = open(where, "r+b", buffering=0)
                break
            except OSError:
                time.sleep(0.2)
        else:
            sys.exit(f"连不上 {where}")
        f.write(payload)
        recv = lambda: f.read(65536)
    else:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(900)
        try:
            s.connect(where)
        except OSError as e:
            sys.exit(f"连不上 {where}：{e}")
        s.sendall(payload)
        recv = lambda: s.recv(65536)
    buf = b""
    while True:
        chunk = recv()
        if not chunk:
            sys.exit("服务断开了，没有回复")
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            msg = json.loads(line)
            if msg.get("id") == 2:
                print(json.dumps(msg["resp"], ensure_ascii=False, separators=(",", ":")))
                return


def patch(src, dst, pairs):
    data = open(src, "rb").read()
    for p in pairs:
        old, new = (x.encode() for x in p.split("=", 1))
        if len(old) != len(new):
            sys.exit(f"长度不同：{old!r} / {new!r}")
        n = data.count(old)
        if n == 0:
            sys.exit(f"{src} 里没有 {old!r}")
        data = data.replace(old, new)
        print(f"{os.path.basename(dst)}：{old.decode()} → {new.decode()}（{n} 处）")
    open(dst, "wb").write(data)
    os.chmod(dst, 0o755)


def make_zip(out, folder):
    top = os.path.basename(os.path.normpath(folder))
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for root, _, files in os.walk(folder):
            for f in sorted(files):
                full = os.path.join(root, f)
                rel = os.path.relpath(full, folder).replace(os.sep, "\\")
                info = zipfile.ZipInfo("x", time.localtime(os.path.getmtime(full))[:6])
                info.filename = f"{top}\\{rel}"
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, open(full, "rb").read())


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    if a[0] == "publish" and len(a) >= 6:
        publish(a[1], a[2], a[3], a[4], a[5:])
    elif a[0] == "ctl" and len(a) == 3:
        ctl(a[1], a[2])
    elif a[0] == "patch" and len(a) >= 4:
        patch(a[1], a[2], a[3:])
    elif a[0] == "zip" and len(a) == 3:
        make_zip(a[1], a[2])
    else:
        sys.exit(__doc__)
