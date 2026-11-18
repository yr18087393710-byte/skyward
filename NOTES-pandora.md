# pandora：解密、触摸、系统应用 —— 结论与未完成项

写这份是因为 2026-11-18 这一天用掉了大约六次完整刷机周期去「猜」一个配置问题，
而真正的差异在运行环境代码里。以后再碰这条线，先读这里，别再刷一次猜一次。

## 三条线（别混）

| 分支 | 底座 | 有解密 | 有 0.3 UI 功能 |
|---|---|---|---|
| `main` @ `91348af` | 0.2.0 + 用户自研功能 | ✅ | ❌ |
| `main-03` @ `757bbc7` | 用户线 + 上游 0.3.0 合并 | ✅ | ❌（UI 仍是 0.2 的扁平 app.rs） |
| `features` @ `0ecc068`… | 上游 0.3.1 移植 | ❌ 差最后一步 | ✅（控制中心/排序/多选/U 盘/二维码/网址下载/测速/镜像自更新） |

`main-03` 是**已知可用**的：`c51ec7a0 0d1a241f cd0d2ba0 aa71425e 14773462 64db6911 7bb3b7c2 e2f407f0`，
刷在上面就能解密、触摸、有系统应用页面。

## 已经搬进 `features` 的（都验证过，别再重复劳动）

1. `crates/skyward-crypto/src/data.rs` —— 换成 0.3.0 那版（凭据 blob 的选取，228 行）。
   随之 `main.rs` 里的 `use_weaver_first` 调用已删除（0.3.0 没有这个开关）。
2. 凭据老路径：`remember.rs` 增加 `OLD_FILE = /mnt/vendor/persist/skyward/unlock`，
   读取时兼读、写入成功后删掉老副本。本机的凭据就存在老路径（4 字节，2026-11-11）。
3. `devices/pandora/system/{bin/se_omapi, lib64/omapi/android.se.omapi-V1-ndk.so, etc/vintf/manifest/se_omapi.xml}`
   ＋ `devices/pandora.toml` 里的 `[[service]] path = "/system/bin/se_omapi"`。
4. **打包器**：`crates/skyward-pack/src/main.rs` 的 `files_in()` 与「设备目录按路径塞进 ramdisk」
   那段（features 移植时被删了 40 行）。没它，第 3 条的文件根本不进镜像
   —— 症状就是 `/system/bin/se_omapi is missing; is its partition mounted?`。
5. `devices/pandora.toml` 的 `[modules] load` 补回 `nxp-nci`（安全元件服务需要它做出的节点）。

## 触摸（已修，features 与 main-03 都适用）

本机触摸层描述两个 direct-multitouch 设备：`Xiaomi_Touch_Input_1`（event7，**不报点**）
和 `Xiaomi_Touch_Input_0`（event8，真正上报）。原代码「先遇到的当面板」会挑到 event7，
于是屏幕对触摸毫无反应。

`crates/skyward/src/input.rs::scan()` 改为：先把候选收集起来，优先取名字以 `_0` 结尾的；
若已把 `_1` 当面板、之后 `_0` 才出现，就换掉（旧 fd 关闭，epoll 自动移除）。

## 未完成：唯一剩下的阻塞

`features` 这条线上，手机的安全元件程序一启动就 `exit status 1`，每秒重试：

```
/vendor/bin/hw/android.hardware.secure_element-service.qti: pid 800 ended: exit status: 1
→ android.hardware.weaver.IWeaver/default is not running
→ 凭据是 weaver 绑定的 → 打不开用户文件
```

同样的配置、同样的文件、同样的 `nxp-nci`，在 `main-03`（0.3.0）上该程序能注册成功：
`is served, with a reader for each of 1 secure elements: eSE1`，SE 崩溃计数 0。

**结论：差异在启动该程序时给的运行环境，即 `crates/skyward/src/services.rs` 与
`crates/skyward-programs`（0.3.0 vs 0.3.1）**：库搜索路径、seclabel、命名空间、继承的 fd。
下一步只做这一件事：把两个 crate 在这两版之间对齐。

## 铁律（省钱的）

1. **先 diff 代码，再出镜像**。CI 一轮约 6 分钟、刷机一轮约 2 分钟，
   盲目刷一次就是十几分钟外加一次真机风险。
2. **能在 WSL 里 `cargo check` 验证的，绝不上手机**。只有「运行环境」这类必须在真机看的，
   才刷。
3. 刷机前先说清这次要验证哪一个日志行；没有可验证的行，就别刷。