# 移动网络（蜂窝数据）在 recovery 里 —— 设计、现状、分几步

2026-11-18 起记。这里只写事实和步骤，写清楚到能直接照着做。

## 进度（2026-11-18 下午，0.3.2 + 之后的提交）

**第一步做成了**：recovery 里 `qcrilNrd` 与 `ssgqmigd` 都起来了，零秒退。

```
/dev/socket/qcrild      建好（我的一段 oneshot）
/dev/socket/qmux_radio  建好（同上）
/dev/socket/ssgqmig     建好（配置里新增的 init socket 支持，seqpacket 0660）
ssgqmigd  pid 719       阻塞在 socket 上（它的空闲态）
qcrilNrd  pid 718       5 个线程，零秒退
modem 固件              Skyward 的 ueventd 从 /firmware/image/peach/amss20.bin 喂给了 mhi
MHI 通道                /sys/bus/mhi/devices/ 里有 ADSP_0..3、IPCR、LOOPBACK
解密                    同版照常（users [0,999] → []）
```

**卡在哪**：`qcrilNrd` 没有连上 QMI。证据是 `/proc/net/unix` 里只有一条
`/dev/socket/ssgqmig`（监听态），**没有客户端**；`qcrilNrd` 也没打开任何 mhi/smdcntl 设备，
只有两个 socket 和它的 DB。

**一个被推翻的假设**：先前以为 qrtr 名字表空就是 modem 没起来。**错**——`qrtr-lookup`
在 Android 里同样是空的（这台机器的 modem 走 glink/smd，不走 QRTR 名字服务）。所以别再用
它判断 modem 死活。

**下一步**：把 Android 里 `qcrilNrd` 的真实 fd 列表和 `/dev/socket/qmux_radio/` 里的内容
抓下来（一次 `adb reboot` 就够），就知道它期望的传输是什么——是 `/dev/socket/qmux_radio`
下的某个 socket、还是 `/dev/smdcntl8`、还是某个属性没设。缺什么补什么。

## 一句话结论

能加，而且比想象中近：**IP 配置那一层 Skyward 已经有了**（`skyward-net::iface::configure` /
`set_up` / `dns` / `reach`，WLAN 就是靠它们上线的）。缺的只有**发起数据呼叫**这一层，
以及让调制解调器的用户态程序在 recovery 里跑起来。

TWRP 一类的 recovery 从来不做移动数据，原因不是做不到，而是没人愿意碰 RIL/QMI。我们这台
有全部条件：分区能读、能起厂商进程、有完整的网络栈。

## 现有资产（不用重写）

| 需要 | 现在有没有 |
|---|---|
| 把接口配 IP / 掩码 / 网关 | ✅ `skyward-net/src/iface.rs`：`configure`、`set_up`、`unconfigure` |
| DNS 解析 | ✅ `skyward-net/src/dns.rs`：`lookup` |
| 测连通性 | ✅ `skyward-net/src/reach.rs` |
| HTTP 下载 / 更新 / 自更新 | ✅ `skyward-net`（WLAN 上已经在用） |
| 起厂商进程（带库路径、用户、组、能力） | ✅ `services.rs` + `pandora.toml` 的 `[[service]]` |
| 从手机分区解析 init 服务 | ✅ `skyward-programs`（`Step::Start`，含 `ready_service`） |

## 缺的四块

### 1. 调制解调器的用户态（recovery 里从没起过）

手机自己的 init 文件（`crates/skyward-programs/tests/described/pandora/`）说得很清楚：

```
service vendor.qcrild /vendor/bin/hw/qcrilNrd
    user radio
    group radio cache inet misc audio log readproc wakelock oem_2901 oem_2912
    capabilities BLOCK_SUSPEND NET_ADMIN NET_RAW
    disabled                     ← 常态下由框架拉起，recovery 里没人拉
on boot
    mkdir /dev/socket/qcrild 0750 radio radio
```

所以第一步是：建 `/dev/socket/qcrild`，再按上面的 user/group/capabilities 起 `qcrilNrd`，
外加它依赖的 QMI 侧程序（`qmiproxy`、`ssgqmigd`、`qmipriod`、`qms` 那一批，逐个试）。

**这一步不需要写 RIL 客户端就能验证**：起来之后看
`/dev/socket/qcrild/qcrild` 是否存在、`qcrilNrd` 是否活着、内核日志里调制解调器有没有注册。
以前我在别的机型上把这一步当"能不能做"的分水岭。

### 2. 发起数据呼叫（真正的新代码）

两条路，二选一：

**A. 走 RIL socket（推荐）** —— `qcrilNrd` 已经把 QMI 的活干完了，我们只要跟它说话：
`socket("/dev/socket/qcrild/qcrild")`，协议是 Android RIL 的老协议：
4 字节长度 + 2 个 int（type、request）+ 参数，字符串是 UTF-16。需要三个请求：

- `RIL_REQUEST_GET_SIM_STATUS`（0x01）—— SIM 在不在、锁没锁
- `RIL_REQUEST_SETUP_DATA_CALL`（0x1B）—— 带 APN 起呼叫，回 `cid` 与地址
- `RIL_REQUEST_DATA_CALL_LIST`（0x38）—— 现在这条呼叫的状态
- 收尾 `RIL_REQUEST_DEACTIVATE_DATA_CALL`（0x29）

代码量估：框架 + parcel 编解码 ≈ 400–600 行，加一个 crate `skyward-ril`。
**不需要**碰 QMI 的二进制协议，风险小一个数量级。

**B. 直接说 QMI** —— 要走 `libqmi_cci` 的传输（`/dev/mhi_*` 或 smd），WDS 服务
（`WDSStartNetworkInterface`、`WDSGetRuntimeSettings`）。协议本身就要上千行，还要自己找
传输通道。除非 A 走不通，不然不做。

### 3. APN

这台机器的描述树里**没有** `apns-conf.xml`（它是 framework 侧的东西）。所以：

- 先做**手输 APN**：一个「移动网络」页面，APN、用户名、密码、鉴权（PAP/CHAP）、IP 类型，
  存在 `/metadata/skyward/mobile`（和 `wlan` 一个待遇，重启还在）。
- 再做**自动**：从装好的系统里读 `apns-conf.xml` / CarrierConfig，按 `mccmnc` 匹配。
  这一步等数据通了再优化，不挡主流程。

### 4. 接口与路由

呼叫通了以后，`qcrilNrd` 会给出 `rmnet_data0` 之类的接口名与地址。之后就是现成的东西：

```
iface::set_up("rmnet_data0", true)?
iface::configure("rmnet_data0", addr, mask, Some(gateway))?
```

DNS 用 `dns::lookup(host, &[dns1, dns2])`，测连通用 `reach::probe`。WLAN 页面上的
下载/更新/自更新全部直接复用，不用改。

## 分四步做，每步都能单独验证

| 步骤 | 做什么 | 怎么算成功 | 需要手机 |
|---|---|---|---|
| 1 | 起 `qcrild` 与 QMI 侧程序 | `/dev/socket/qcrild/qcrild` 出现；`qcrilNrd` 活着不重启；日志里 modem 已注册 | 是 |
| 2 | `skyward-ril` crate：framing + `GET_SIM_STATUS` | 读出 SIM 状态（在不在、运营商） | 是 |
| 3 | `SETUP_DATA_CALL` + 页面（手输 APN） | 拿到 `cid` 与地址，`rmnet_data0` 有 IP，能 ping 通 223.5.5.5 | 是 |
| 4 | 接入更新/WLAN 页的下载路径，做自动 APN | 拔了 WiFi 也能检查更新 | 是 |

估时（老实说）：1 约半天，2 约一天，3 约一到两天，4 约半天。**每步都要真机**，因为调制解调器
的行为在 QEMU 里没有。

## 风险（先说清，不藏）

1. **调制解调器固件**：`modem` 分区由内核/modem 自己加载，recovery 通常不碰它，但某些平台要
   用户态喂固件（`rmtfs`）。若不喂，modem 起不来 —— 这一步失败就没有后面。
2. **SELinux**：recovery 一般是 permissive，理论上不挡；真挡了就是域名/标签问题。
3. **`/data` 没解**时 modem 的 NV/EFS 在某些机型上要读写 `/data/vendor/radio`，可能需要
   `/vendor/firmware_mnt` 与 `/mnt/vendor/persist` 都挂着（Skyward 已经挂 persist）。
4. **双卡**：`qcrilNrd -c 2` / `-c 3` 是另外两条 RIL 实例，先只做卡 1。

## 下一步

第一步不需要新代码，只需要往 `devices/pandora.toml` 加几段 `[[service]]` 并在 recovery 里
验证「modem 起没起来」。要做的时候说一声，我先只加这一段、只验这一件事，不碰网络配置。