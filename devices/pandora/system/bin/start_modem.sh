#!/vendor/bin/sh
# 让调制解调器开机。
#
# 内核用 request_firmware 向 ueventd 要 modem.mdt 和它的一串 modem.b*，而 ueventd
# 只在 /etc/firmware、/odm/firmware、/vendor/firmware、/firmware/image 里找。
# 这台机器的 modem 固件在 modemfirmware 分区里（分区内是一层 image/ 目录），
# 恢复系统不挂它，于是内核报 "request_firmware failed: -2"、modem 永远 offline。
#
# 整个分区还要放在 /vendor/modem_firmware：原厂 RFS 的 readonly/firmware
# 指向这里，TFTP 会从中读取 modem_oem/qdsp6_oemsw.mbn。
# 再把 image 目录绑到 ueventd 会找的地方，最后让内核启动 modem。
# 起来之后 logs 里是：remote processor 4080000.remoteproc-mss is now up

set -u

# Diagnostic only; the recovery configuration must not auto-start MSS.
if [ "$(getprop init.svc.recovery)" != "running" ]; then
    echo "start_modem: run this only in recovery" >&2
    exit 1
fi
rproc=/sys/class/remoteproc/remoteproc2
case "$(readlink -f "$rproc")" in
    */4080000.remoteproc-mss/remoteproc/remoteproc2) ;;
    *) echo "start_modem: remoteproc2 is not the expected MSS device" >&2; exit 1 ;;
esac
if [ "$(cat "$rproc/state")" != "offline" ]; then
    echo "start_modem: expected the offline MSS remote processor" >&2
    exit 1
fi

slot=$(getprop ro.boot.slot_suffix)
case "$slot" in
    _a|_b) ;;
    *) echo "start_modem: unknown boot slot: $slot" >&2; exit 1 ;;
esac

mode=${1:-start}
case "$mode" in
    start|--stage-only) ;;
    *) echo "start_modem: expected start or --stage-only" >&2; exit 1 ;;
esac

part="/dev/block/by-name/modemfirmware$slot"
canonical=/vendor/modem_firmware
# Refuse to cover mounts from an earlier diagnostic run.
if grep -q ' /vendor/modem_firmware ' /proc/mounts || grep -q ' /etc/firmware ' /proc/mounts; then
    echo "start_modem: firmware staging mounts already exist" >&2
    exit 1
fi
mnt=$(mktemp -d /mnt/modemfirmware.XXXXXX) || exit 1
if ! mount -t vfat -o ro "$part" "$mnt"; then
    rmdir "$mnt"
    exit 1
fi
if [ ! -r "$mnt/image/modem.mdt" ] || [ ! -r "$mnt/image/modem_oem/qdsp6_oemsw.mbn" ] || ! mkdir -p "$canonical" /etc/firmware; then
    echo "start_modem: missing required firmware or mount directory" >&2
    umount "$mnt" && rmdir "$mnt"
    exit 1
fi
if ! mount --bind "$mnt" "$canonical"; then
    umount "$mnt" && rmdir "$mnt"
    exit 1
fi
if ! mount --bind "$mnt/image" /etc/firmware; then
    umount "$canonical" && umount "$mnt" && rmdir "$mnt"
    exit 1
fi
rfs=/vendor/rfs/msm/mpss/readonly/firmware/image/modem_oem/qdsp6_oemsw.mbn
if [ ! -r "$rfs" ]; then
    echo "start_modem: RFS cannot resolve OEM firmware" >&2
    umount /etc/firmware && umount "$canonical" && umount "$mnt" && rmdir "$mnt"
    exit 1
fi
if [ "$mode" = --stage-only ]; then
    if ! umount /etc/firmware || ! umount "$canonical" || ! umount "$mnt" || ! rmdir "$mnt"; then
        echo "start_modem: firmware path verified but staging cleanup failed" >&2
        exit 1
    fi
    echo "start_modem: firmware paths verified and staging cleaned; MSS remains offline"
    exit 0
fi
if ! printf '%s\n' start > "$rproc/state"; then
    umount /etc/firmware && umount "$canonical" && umount "$mnt" && rmdir "$mnt"
    exit 1
fi