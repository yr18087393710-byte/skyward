#!/system/bin/sh
# The secure element HAL, running on the recovery's own libraries. The NXP
# NFC/eSE driver is not among the modules the recovery loads, and the chip
# has to answer before the HAL's TA can come up, so load it first and bring
# the HAL back until it stays.

insmod /vendor/lib/modules/nxp-nci.ko 2>/dev/null
i=0
while [ ! -e /dev/nq-nci ] && [ $i -lt 30 ]; do
    sleep 1
    i=$((i + 1))
done

export LD_LIBRARY_PATH=/system/lib64:/vendor/lib64:/vendor/lib64/hw:/odm/lib64:/skyward/lib64
i=0
while [ $i -lt 900 ]; do
    /skyward/hw/se-qti && exit 0
    i=$((i + 1))
    sleep 2
done
exit 1
