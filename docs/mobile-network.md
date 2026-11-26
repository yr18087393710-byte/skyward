# 移动网络（蜂窝数据）在 recovery 里 —— 设计、现状、分几步

2026-11-18 起记。这里只写事实和步骤，写清楚到能直接照着做。

## 最新（2026-11-19）：存储读通，蜂窝数据未通

- 原厂 Android 在 `_b` 槽把 `modem_b` 挂到 `/vendor/firmware_mnt`，把 `modemfirmware_b` 挂到 `/vendor/modem_firmware`。两槽的 `modem.mdt` 摘要相同；没有据此断言所有 `modem.b*` 相同。
- Recovery 用 `_b` 槽固件并先启动原厂 `rmt_storage`、`tftp_server` 后，日志记录了 MSS `running`、`QRTR: Received HELLO from node 0`、两个 modem FS 卷的成功读取，以及内核 coresight 的 QMI 连接。这证明部分通信已发生，不代表用户态 radio QMI 就绪。
- 早期一轮 `qrtr-lookup` 仍只有表头、`Modemcomm` 仍返回 `-3`，手机随后重启进入 Android；该轮日志截断，未抓到触发原因。img28 的独立主机采集补齐了故障证据：2026-11-19 11:45–11:46（Asia/Shanghai），MSS 在 uptime `228.943795` 秒启动，`228.973357` 秒收到 node 0 HELLO，`229.100148` 秒完成 modem_fs2 的 4,193,280 字节读取；`269.952059` 秒报告 `dog_hal_common.c:178:DOG detects stalled initialization, triage with IMAGE OWNER`。内核同时记录 remoteproc recovery disabled，随后触发 device crash。已确认 modem 初始化 watchdog 是该轮崩溃触发链，尚未定位哪个初始化依赖停滞。
- 原厂 `hlosminkdaemon.rc` 把 `ssgtzd` 和 `hlos_mink_opener` 都交给 `hlosminkdaemon`。恢复配置把它们放在已经存在的 `hlosminkdaemon` 服务下，沿用原厂所有者与权限；`ssgqmigd` 保留自己的 `ssgqmig`。这是可验证的配置差异，尚未证明它解释 `-3` 或重启。
- Recovery 配置让 `rmt_storage`、`tftp_server` 和 `hlosminkdaemon` 先于 radio clients 启动，并保持已验证的 `qseecomd → hlosminkdaemon → keymint` 解密服务顺序；modem 自动启动仍关闭。已用原厂底座和已验证的 img26 二进制生成配置测试镜像 img28（SHA-256 `1cea0b68944d6e5775f698a90ebbe5fc3d629777264548d8d5e674e5f19056a6`），刷入 `recovery_b`。基线采集记录至 uptime 178 秒，手动启动前 uptime 已到 228 秒：MSS 保持 offline、四个 daemon 在运行、Mink 两个监听 inode 由 `hlosminkdaemon` 持有，用户文件约 7.22 秒全部解密。手动启动后九次采样中 MSS 为 running、daemon PID 未变，但 `qrtr-lookup` 仍只有表头，约 41 秒后出现上述 modem 初始化 watchdog 崩溃。随后 ADB 重连确认 Android `sys.boot_completed=1`、槽位 `_b`；`ro.boot.bootreason`、`sys.boot.reason` 和 `sys.boot.reason.last` 均包含 `kernel_panic,panicking,_remoteproc_4080000`，与主机记录一致。完整证据位于主机 `tmp/recovery-capture-20261119-114508/`（`epoch-1-dmesg.txt` 第 3308–3315 行是故障边界）；基线为 `tmp/recovery-capture-20261119-114131/`，重启后属性为 `tmp/img28-post-crash-android.txt`。先验证 `ssgtzd`/`hlos_mink_opener` 的单一监听者与解密回归，再做带主机侧连续内核日志的单次 modem 启动。仅在 radio QMI 持续可用后测试数据呼叫，不以 `running` 或 QRTR HELLO 代替验收。

### 12:00 路径排查：RFS 找不到 OEM 固件

- img28 的完整 logcat 第 16382–16387 行显示 modem 请求 `readonly/firmware/image/modem_oem/qdsp6_oemsw.mbn`，TFTP 将路径解析为 `/vendor/rfs/msm/mpss/readonly/firmware/image/modem_oem/qdsp6_oemsw.mbn` 并返回 `ENOENT`。Android 可读的 symlink 明确是 `/vendor/rfs/msm/mpss/readonly/firmware → /vendor/modem_firmware`；原手动脚本仅把分区挂到临时目录、把 `image/` 绑到 `/etc/firmware`，因此 RFS 的原厂路径缺失。
- Recovery、MSS offline 状态下，只读挂载当前槽确认文件存在，大小 21,597,436 字节，SHA-256 `8adefa6655edd057d463dd1e587c47c59e187165454c801e71e199737cf695ec`。修改 helper 为同时 bind 整个分区到 `/vendor/modem_firmware`，保留 `image/ → /etc/firmware`；加入 `--stage-only` 验证模式和重复 staging 拒绝。
- 把新 helper 推到 `/tmp` 后执行 `--stage-only`，MSS 读回仍为 offline；经原厂 RFS 路径读回文件的 SHA-256 与上述一致。这确认修复了路径可达性，尚未证明解决 modem 初始化 watchdog。
- 后续受控启动命令在 push 探针时即因设备未连接而退出，没有执行新的 MSS start。USB ADB/fastboot 均未枚举，新的主机 capture 从首个样本起就没有连接；离线原因和当前手机启动状态未确定，不能把这次当作新的 modem 测试结果。已刷镜像仍是 img28，仓库 helper 的新改动只推到了临时 `/tmp`，没有重新刷入。
- 隔离 shell 回归测试已整理到 `devices/pandora/tests/test_modem_staging.py`，使用 Python 标准库 `unittest`，覆盖两个槽位、stage-only 不启动 MSS、正常 start、挂载失败及失败清理、缺失 OEM 固件、已有挂载、无效 mode/slot、非 Recovery、错误 remoteproc 和 MSS 已运行，共 13 个场景通过。测试只在临时目录替换 helper 路径并模拟 mount，不触碰主机或设备挂载。已把该测试接入 `.github/workflows/build.yml` 的 workspace tests 后。

### img29：已打包路径修复，待设备验证

- 生成 `D:/fbwork/flash/img29/skyward-pandora-rfs-path-manual.img`，104,857,600 字节，SHA-256 `8d37623602fd171ab7da8e237a7b9c4a11106aa12fb6436d80d6f1e6cb23bfad`。未刷入，设备仍未通过 USB ADB/fastboot 枚举；不能报告 img29 的启动、解密、TFTP 或 QMI 成功。
- 打包前校验原厂 recovery 摘要和 img26 复用二进制摘要；打包后 `cat` 取出配置、helper、recovery、system-init，全部与输入 `cmp` 一致。镜像 helper SHA-256 `a52075cdb7f66c1d4a80b364a7fc8a055ba5798f2f66f288351967fdd0cbf694`；配置确认无 modem 自动启动、secure-world 顺序保持。
- 镜像脚本语法和 9 项隔离行为测试通过。复现脚本 `target/pack-pandora-rfs-path-test.sh`；各文件摘要 `target/pandora-rfs-path-verification/SHA256SUMS`。设备重连后先识别当前模式/槽位，再进行 offline staging 检查；任何 MSS 启动都先开启主机日志，检查 TFTP OEM 固件读取、超过既往 41 秒边界的稳定性和实际 QRTR 服务表。

### img30：MSS 越过 watchdog，蜂窝数据仍未通

- img30 修正 `--stage-only` 验证后的临时挂载清理；配置仍关闭 MSS 自动启动。镜像 SHA-256 `eb6e777b81079c730d9373986c88b1030a98f613627a2d4a8d06d6ecab0886f1`。本地 26 项配置测试及 13 个隔离 staging 场景通过。已确认 `pandora` 槽位 `b`，刷入 `recovery_b`。
- 启动前 Recovery 中 MSS 为 `offline`，离线核对 `_b` 分区 OEM 文件摘要。仅手动启动一次；从 uptime `167.86` 到 `472.49` 秒的 61 个样本均为 `running`，RFS/Mink/`ssgqmigd` PID 未变。到 uptime `1130.34` 秒仍为 `running`、PID 未变，距手动启动约 16 分钟；本次未复现 img28 的约 41 秒重启。该结果仅覆盖一次启动。
- 主机 `tmp/img30-tftp-raw.txt` 第 140–148 行：同一 TFTP 请求线程 1648 打开 OEM 固件、传输 8,204,928 字节并报告成功。这确认原厂 RFS 路径可用于实际传输，不代表单笔传完 21,597,436 字节的整个文件。采样日志为 `tmp/img30-mss-samples.txt`。
- `Qms Ready` 出现在启动 MSS 后；`Modemcomm -3` 和连接 SCA 失败出现在 MSS 启动前，不能据此判定启动后的 SCA 状态。启动后通配和定向 QRTR 查询（WDS=1、NAS=3）仍只有表头；`/dev/socket/qcrild` 为空，未见蜂窝数据接口。尚未验收持续的 radio QMI 请求应答或数据呼叫。原厂在 `sys.boot_completed=1` 时调用 `qrtr-lookup`；本轮已在 Recovery 显式执行，差异仍待定位。
- MSS 的 `glink-edge.IPCRTR` 设备已绑定 `qcom_smd_qrtr`。ADB shell、TFTP、Mink、`qms` 和 `qcrilNrd` 的网络命名空间均为 `net:[4026531840]`。内核 `qrtr` 模块包含 `qrtr_ns_init` 与 `qrtr_ns_worker`，但符号存在并不证明注册正常。复测挂载 tracefs 后，启动前离线查询产生了 `new-lookup`/`del-client`；手动启动后收到 `hello from 0:-2`，并记录了远端服务注册：`[1:1]@[0:189]`、`[3:1]@[0:193]`，随后还有多个服务注册和 `new-lookup`。这说明 MSS/QRTR 名称服务已经看到 WDS/NAS 服务；同一时刻 `qrtr-lookup` 仍只打印表头，因此该工具输出与已记录服务注册不一致，不能再作为服务不存在的证据。Recovery 内无 framework，因此未启动 `qcrilNrd`，`/dev/socket/qcrild` 与 `/dev/socket/qmux_radio` 为空；`qmipriod`、`qms`、`nicmd`、`ssgqmigd` 则观察为运行中。已对设备自带 `/vendor/bin/qmi-framework-tests/qmi_test_mt_client_init_instance` 做离线字符串检查，按帮助对 WDS service id 1 跑单线程、单次、2 秒限时 `-i 1 -n 1 -d 2000`，客户端 init 两个线程都返回 `rc -3`，工具自身打印 `PASS` 仅表示测试 harness 跑完。未执行 `-q`，它是切换 QMUX transport 的选项，不是给默认 QRTR 测试加一个无害 flag。MSS 仍 running，测试后 daemon PID 未变。结论：QRTR 远端注册已有直接 trace 证据，但 QMI CCI client init 仍失败；优先比较 Recovery 与 Android 的服务启动属性、设备服务配置及 `libqmi_cci` transport 参数。仅在真实 QMI 应答后测试数据呼叫。

- 最后一次在线读取为 Recovery uptime `1294.51` 秒，MSS 仍为 `running`，距启动约 18 分 47 秒。之后 USB 断开并重新枚举为 Android（ADB transport 18，`sys.boot_completed=1`，槽 `_b`）。`ro.boot.bootreason`、`sys.boot.reason`、`sys.boot.reason.last` 均为 `bootloader`，不同于 img28 的 `kernel_panic`；没有覆盖退出时刻的内核日志，Android ADB 也无权读取 pstore，退出原因未定。Android 侧 MSS 的 IPCRTR 同样绑定 `qcom_smd_qrtr`，并暴露 `qrtr_ns_message`、`qrtr_ns_server_add` 等 tracepoint。复测沿用 img30：在独立 trace 实例中先做离线查询基线，再在一次手动启动前开启有限采集，比较本地 `new-lookup` 和启动期的远端服务注册。

## 历史记录（2026-11-18）：首次启动 MSS

以下保留早期排查过程。涉及 MHI、QRTR 和 socket 所属的旧推断以上方实测与原厂定义为准；其中 `mhi_110e_00.01.00` 属于 WLAN，并非蜂窝 modem。

**modem 在恢复系统里开机了。** 这是这台机器上第一次：

```
mount --bind /mnt/modemfirmware/image /etc/firmware
echo start > /sys/class/remoteproc/remoteproc2/state

[ 5.828754] remoteproc remoteproc2: Booting fw image modem.mdt, size 7252
[ 6.196125] remoteproc remoteproc2: remote processor 4080000.remoteproc-mss is now up
```

原因是：modem 固件在 `modemfirmware_a/_b` 分区里（分区内是一层 `image/`），
而内核通过 ueventd 找 `modem.mdt` 时只搜 `/etc/firmware`、`/odm/firmware`、
`/vendor/firmware`、`/firmware/image`——恢复系统一个都没挂，于是
`request_firmware failed: -2`、modem 永远 `offline`。

脚本在 `devices/pandora/system/bin/start_modem.sh`。

**但它在配置里是注释掉的**，因为：modem 起来后 **60～70 秒整机会自己重启**
（此处是首次自动启动试验的现象；触发原因未确认）。后续试验补齐存储服务后仍出现重启，见上文。
判据：modem 起来后 `qrtr-lookup` 里出现服务、`qcrilNrd` 不再报
`qmi_client_init_instance status: -3`、`/dev/socket/qcrild/rild0` 出现。

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

`qrtr-lookup` 空表不能单独判断 modem 是否运行。原厂 Android 上普通 ADB shell 无权执行这个程序，不能把 `Permission denied` 当成空表；Recovery 上后来同时观测到了 QRTR HELLO 和空表。

**下一步**：把 Android 里 `qcrilNrd` 的真实 fd 列表和 `/dev/socket/qmux_radio/` 里的内容
抓下来（一次 `adb reboot` 就够），就知道它期望的传输是什么——是 `/dev/socket/qmux_radio`
下的某个 socket、还是 `/dev/smdcntl8`、还是某个属性没设。缺什么补什么。

## 后来的探测（2026-11-18 傍晚，结论：这条路今天走不通）

`qcrilNrd` 在 recovery 里能完整初始化它的 AIDL 模块（RILQ、RadioConfig、Messaging、
Modem、Network、Sim、Voice、UimRemote、Lpa 全部注册），然后**卡死在
`QmiClientModule` 的初始化里**：日志停在注册 `QMI_CLIENT_CHECK_IF_SERVICE_UP` 之后，
一共 600 行，不再往前走，也不报错。它既不建 `/dev/socket/qcrild/rild0`，也不连
`/dev/socket/ssgqmig`（`/proc/net/unix` 里只有监听态，没有客户端）。

补过的其它守护进程：

| 程序 | 结果 |
|---|---|
| `ssgqmigd` | ✅ 起来，socket `/dev/socket/ssgqmig` 建好（配置里新增的 init socket 支持） |
| `qmipriod` | ✅ 起来（它的字符串里有 `rmnet`、`wds`） |
| `nicmd` | ❌ `CANNOT LINK EXECUTABLE: library "libcom.android.tethering.connectivity_native.so" not found` |
| `qms` | ❌ 同一条库缺失 |
| `servicemanager` | ✅ 本来就在跑（pid 524），不是瓶颈 |

`nicmd` 与 `qms` 缺的库来自 Android 的 Connectivity **APEX**，recovery 里根本没有那个 APEX。
也就是说：**Android 的 telephony 用户态在 recovery 里缺零件，不是差一个配置**。

剩下唯一可行的路是自己当 QMI 客户端（连 `/dev/socket/ssgqmig` 发 WDS 请求），
但那个 socket 上的帧格式是 Qualcomm 私有的（libqmi_cci 的 "ssg" 传输），
没有文档，需要拿真机做逆向才能确定 —— 这是一项独立工程，不是一次会话能完成的。

**电话**同理且更远：需要 RIL 起来 + 音频 HAL + 通话状态机。

## 一句话结论

能加，而且比想象中近：**IP 配置那一层 Skyward 已经有了**（`skyward-net::iface::configure` /
`set_up` / `dns` / `reach`，WLAN 就是靠它们上线的）。缺的只有**发起数据呼叫**这一层，
以及让调制解调器的用户态程序在 recovery 里跑起来。

TWRP 一类的 recovery 从来不做移动数据，原因不是做不到，而是没人愿意碰 RIL/QMI。我们这台
有全部条件：分区能读、能起厂商进程、有完整的网络栈。

## 2026-11-18 傍晚：启动 MSS 前的 QRTR 跟踪

当时 `qcrilNrd` 发出了 QRTR 查询，但在跟踪窗口内没有收到回复：

```
qcrilNrd 建 AF_QIPCRTR socket 后，向所有节点广播 QRTR 查询：
  sendto(8, "\n\0\0\0\10\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", 20, 0,
         {sa_family=AF_QIPCRTR, sa_data="\0\0\1\0\0\0\376\377\377\377"})   ← 0xFFFFFFFE = 广播
没有任何 recvfrom 回答。
```

这个跟踪只说明当时没有收到回复。后续手动启动 MSS 后，内核收到 QRTR HELLO，但 `qrtr-lookup` 仍只有表头；两者观察的是不同阶段。`/firmware/image/peach/amss20.bin` 的 MHI 加载属于 WLAN，不能用来判断蜂窝 modem 是否启动。

`nicmd` / `qms` 的历史链接问题涉及 `libcom.android.tethering.connectivity_native.so`、`libandroid_net.so` 和 `libnetd_client.so`。动态库可加载不等于 radio QMI 已经可用。

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