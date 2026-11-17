#!/system/bin/sh
# The NXP weaver behind the phone's secure element. Its transport opens the
# element as the program comes up, so it waits for the secure element HAL
# instead of aborting once and staying down.
export LD_LIBRARY_PATH=/skyward/lib64:/odm/lib64:/vendor/lib64:/vendor/lib64/hw:/system/lib64
export LD_PRELOAD=/skyward/lib64/libbinder.so
i=0
while [ $i -lt 900 ]; do
    /skyward/hw/weaver-nxp && exit 0
    i=$((i + 1))
    sleep 2
done
exit 1
