// Validates btshim against real hardware, the way Rust will drive it:
// every IOBluetooth call from a non-main thread, marshalled onto the shim's
// own run loop thread. If this works, the Rust binding will too.
#include <stdio.h>
#include <string.h>
#include <pthread.h>
#include <stdlib.h>
#include "btshim.h"

static const uint8_t MAESTRO_UUID[16] = {
    0x25, 0xe9, 0x7f, 0xf7, 0x24, 0xce, 0x4c, 0x4c,
    0x89, 0x51, 0xf7, 0x64, 0xa7, 0x08, 0xf7, 0xb5,
};

static void *work(void *unused) {
    (void)unused;
    setvbuf(stdout, NULL, _IOLBF, 0);   // bt_run_loop holds main, so flush as we go
    printf("[worker thread] driving the shim\n");

    bt_device_t devices[16];
    int32_t n = bt_list_devices(devices, 16);
    printf("bt_list_devices -> %d\n", n);
    if (n < 0) { printf("FAIL: bluetooth unavailable\n"); exit(1); }

    const char *addr = NULL;
    for (int i = 0; i < n; i++) {
        printf("  %-28s %s%s\n", devices[i].name, devices[i].addr,
               devices[i].connected ? "  CONNECTED" : "");
        if (strstr(devices[i].name, "Pixel Buds") && devices[i].connected) addr = devices[i].addr;
    }
    if (!addr) { printf("FAIL: no connected Pixel Buds\n"); exit(1); }

    int32_t cid = bt_rfcomm_channel_for_uuid(addr, MAESTRO_UUID);
    printf("bt_rfcomm_channel_for_uuid -> %d\n", cid);
    if (cid < 0) { printf("FAIL: no maestro channel\n"); exit(1); }

    int32_t err = 0;
    bt_chan_t *chan = bt_rfcomm_open(addr, (uint8_t)cid, &err);
    printf("bt_rfcomm_open -> %s (err=0x%08x)\n", chan ? "OPEN" : "NULL", err);
    if (!chan) { printf("FAIL: could not open from a worker thread\n"); exit(1); }
    printf("bt_mtu -> %u\n", bt_mtu(chan));

    size_t total = 0;
    for (int round = 0; round < 10; round++) {
        uint8_t buf[512];
        int32_t got = bt_recv(chan, buf, sizeof buf, 500);
        if (got < 0) { printf("channel closed\n"); break; }
        if (got == 0) continue;
        total += (size_t)got;
        printf("bt_recv -> %d bytes:", got);
        for (int i = 0; i < got && i < 24; i++) printf(" %02x", buf[i]);
        printf("%s\n", got > 24 ? " ..." : "");
    }
    printf("total %zu bytes received\n", total);
    bt_close(chan);
    printf("%s\n", total > 0 ? "PASS: shim works from a worker with live data" : "PARTIAL: opened but no data");
    exit(0);
}

int main(void) {
    bt_init();
    pthread_t t;
    pthread_create(&t, NULL, work, NULL);
    bt_run_loop();   // never returns; the worker calls exit()
    return 0;
}
