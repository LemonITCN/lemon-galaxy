# uniffi 生成代码

`galaxy_ffi.swift` 由仓库根的 `scripts/build-ffi-apple.sh` 生成（与 `Vendor/LemonGalaxyCore.xcframework` 一起），不要手改。
重新生成后运行 `python3 scripts/gen-bridge-mirror.py` 同步无内核镜像，见 `../Bridge/README.md`。
