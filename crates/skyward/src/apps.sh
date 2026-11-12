#!/system/bin/sh
# Back up and restore the apps the phone keeps in its system partitions.
#
# The recovery mounts those partitions as it starts, at /mnt/system,
# /mnt/system_ext, /mnt/product and /vendor. An app is a folder holding its
# apk, so a backup keeps those folders as they are, under the place backups
# go, and a restore puts them back.

STORE=${STORE:-/sdcard/Skyward/apps}
BASES="/mnt/system /mnt/system_ext /mnt/product /vendor"

say() { echo "$@"; }

find_apps() {
    for base in $BASES; do
        [ -d "$base" ] || continue
        for kind in app priv-app; do
            for dir in "$base/$kind"/*; do
                [ -d "$dir" ] || continue
                set -- "$dir"/*.apk
                [ -e "$1" ] || continue
                echo "$dir"
            done
        done
    done
}

size_of() {
    du -sk "$1" 2>/dev/null | cut -f1
}

total_size() {
    find_apps | while read -r dir; do size_of "$dir"; done | awk '{s+=$1} END {print s+0}'
}

list_apps() {
    n=0
    find_apps > /tmp/skyward-apps.list
    while read -r dir; do
        n=$((n+1))
        kb=$(size_of "$dir")
        printf "%3d  %-46s %6s KB\n" "$n" "${dir}" "$kb"
    done < /tmp/skyward-apps.list
    say ""
    say "共 $n 个系统应用，合计 $(( $(total_size) / 1024 )) MB"
}

# Which ones were picked: numbers, ranges with -, or all.
pick() {
    : > /tmp/skyward-apps.pick
    case "$1" in
        all|"") cat /tmp/skyward-apps.list > /tmp/skyward-apps.pick; return 0 ;;
    esac
    for part in $(echo "$1" | tr ',' ' '); do
        case "$part" in
            *-*)
                from=${part%-*}; to=${part#*-}
                sed -n "${from},${to}p" /tmp/skyward-apps.list >> /tmp/skyward-apps.pick
                ;;
            *)
                sed -n "${part}p" /tmp/skyward-apps.list >> /tmp/skyward-apps.pick
                ;;
        esac
    done
    [ -s /tmp/skyward-apps.pick ]
}

backup() {
    picked=$1
    pick "$picked" || { say "没有选中任何应用"; return 1; }
    count=$(wc -l < /tmp/skyward-apps.pick)
    say "备份 $count 个应用到 $STORE"
    say ""
    done_n=0
    while read -r dir; do
        done_n=$((done_n+1))
        target="$STORE$dir"
        mkdir -p "$(dirname "$target")" || { say "建不了目录: $(dirname "$target")"; continue; }
        printf "[%d/%d] %s ... " "$done_n" "$count" "$dir"
        if cp -a "$dir" "$target" 2>/dev/null; then
            echo "好"
        else
            echo "失败"
        fi
    done < /tmp/skyward-apps.pick
    say ""
    say "备份完成，在 $STORE"
}

# The partition an app lives in, mounted writable for the copy and read only
# again after it.
writable() {
    base=$1
    if mount | grep -q " $base "; then
        mount -o remount,rw "$base" 2>/dev/null && return 0
        return 1
    fi
    return 1
}

restore() {
    picked=$1
    [ -d "$STORE" ] || { say "还没有备份过: $STORE 不存在"; return 1; }
    find "$STORE" -name '*.apk' -type f > /tmp/skyward-apps.restorable 2>/dev/null
    if [ ! -s /tmp/skyward-apps.restorable ]; then
        say "$STORE 里没有备份"
        return 1
    fi
    : > /tmp/skyward-backup.list
    sed 's|^'"$STORE"'||; s|/[^/]*$||' /tmp/skyward-apps.restorable | sort -u > /tmp/skyward-backup.list
    n=0
    say "可恢复的应用："
    while read -r dir; do
        n=$((n+1))
        printf "%3d  %s\n" "$n" "$dir"
    done < /tmp/skyward-backup.list
    say ""
    printf "选哪些（如 1 3 5-7 / all）: "
    read -r which
    cp /tmp/skyward-backup.list /tmp/skyward-apps.list
    pick "$which" || { say "没有选中"; return 1; }
    say ""
    while read -r dir; do
        base=$(echo "$dir" | cut -d/ -f1-2)
        [ "$base" = "/mnt/system_ext" ] && base=/mnt/system_ext
        mountpoint=
        for b in $BASES; do
            case "$dir" in "$b"/*) mountpoint=$b ;; esac
        done
        if [ -z "$mountpoint" ]; then
            say "跳过（不认识的分区）: $dir"
            continue
        fi
        printf "%s ... " "$dir"
        if writable "$mountpoint"; then
            if cp -a "$STORE$dir/." "$dir/" 2>/dev/null; then
                echo "好"
            else
                echo "失败"
            fi
            mount -o remount,ro "$mountpoint" 2>/dev/null
        else
            echo "分区只读，挂不上去"
        fi
    done < /tmp/skyward-apps.pick
    say ""
    say "恢复完成"
}

menu() {
    say "系统应用备份 / 恢复"
    say "备份在: $STORE"
    say "======================================"
    list_apps
    say ""
    say "  1) 备份选中的应用"
    say "  2) 从备份恢复"
    say "  3) 刷新列表"
    say "  0) 退出"
    say ""
    printf "选: "
    read -r choice
    case "$choice" in
        1)
            printf "备份哪些（如 1 3 5-7 / all）: "
            read -r which
            backup "$which"
            ;;
        2) restore "" ;;
        3) menu ;;
        0) exit 0 ;;
        *) say "不认识: $choice" ;;
    esac
    say ""
    printf "按回车继续…"
    read -r dummy
    menu
}

menu
