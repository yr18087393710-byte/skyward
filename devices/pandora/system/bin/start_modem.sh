#!/vendor/bin/sh
# 让调制解调器开机。
#
# 内核用 request_firmware 向 ueventd 要 modem.mdt 和它的一串 modem.b*，而 ueventd
# 只在 /etc/firmware、/odm/firmware、/vendor/firmware、/firmware/image 里找。
# 这台机器的 modem 固件在 modemfirmware 分区里（分区内是一层 image/ 目录），
# 恢复系统不挂它，于是内核报 "request_firmware failed: -2"、modem 永远 offline。
#
# 这里把那个 image 目录绑到 ueventd 第一个会找的地方，再让内核把 modem 拉起来。
# 起来之后 logs 里是：remote processor 4080000.remoteproc-mss is now up

set -u

mkdir -p /etc/firmware /mnt/modemfirmware

if [ ! -e /etc/firmware/modem.mdt ]; then
    for slot in a b; do
        part="/dev/block/by-name/modemfirmware_$slot"
        [ -e "$part" ] || continue
        mount -t vfat "$part" /mnt/modemfirmware 2>/dev/null || continue
        if [ -d /mnt/modemfirmware/image ]; then
            mount --bind /mnt/modemfirmware/image /etc/firmware 2>/dev/null && break
        fi
    done
fi

if [ ! -e /etc/firmware/modem.mdt ]; then
    echo "start_modem: the modem's firmware is not in /etc/firmware, so it stays offline" >&2
    exit 1
fi

echo start > /sys/class/remoteproc/remoteproc2/state