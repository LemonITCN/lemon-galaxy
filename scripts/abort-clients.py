#!/usr/bin/env python3
"""模拟客户端中途断开：建 TLS 连接、一次塞进一串请求，不读响应就用 RST 掐掉，重复若干次。
服务器正在写响应时对端已经断开，用来验证服务器不会被 SIGPIPE 杀掉（见 scripts/e2e.sh）。
  abort-clients.py <主机> <端口> [次数]"""
import socket, ssl, struct, sys, time

host, port = sys.argv[1], int(sys.argv[2])
rounds = int(sys.argv[3]) if len(sys.argv) > 3 else 30
ctx = ssl.create_default_context()
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
ctx.set_alpn_protocols(["http/1.1"])
req = f"GET / HTTP/1.1\r\nHost: {host}\r\n\r\n".encode() * 200
done = 0
for i in range(rounds):
    try:
        raw = socket.create_connection((host, port), timeout=5)
        s = ctx.wrap_socket(raw, server_hostname="lemongalaxy")
        s.sendall(req)
        time.sleep(0.02)
        # SO_LINGER 0：关闭时直接发 RST，服务器后面的写都会碰到已断开的连接
        s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
        s.close()
        done += 1
    except OSError as e:
        print(f"第 {i + 1} 次连接失败：{e}")
        break
    time.sleep(0.02)
print(f"中途断开的连接：{done}/{rounds}")
sys.exit(0 if done == rounds else 1)
