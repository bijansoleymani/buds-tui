// C ABI over macOS IOBluetooth, for the Rust side of the buds-tui macOS port.
//
// IOBluetooth is delegate- and run-loop-driven, which does not map onto Rust
// async at all. So this shim owns one dedicated thread running a CFRunLoop,
// marshals every IOBluetooth call onto it, and buffers inbound data so Rust
// can read it with an ordinary blocking call it can put on a blocking pool.

#ifndef BUDS_BTSHIM_H
#define BUDS_BTSHIM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BT_ADDR_LEN 18
#define BT_NAME_LEN 64

typedef struct {
    char name[BT_NAME_LEN];
    char addr[BT_ADDR_LEN];   // "xx-xx-xx-xx-xx-xx"
    bool connected;
} bt_device_t;

// Idempotent; safe to call from any thread.
void bt_init(void);

// Gives the calling thread to a run loop, forever. Must be called on the main
// thread: IOBluetooth only opens channels from main, and only delivers its
// callbacks to a running run loop. Everything else must live on other threads.
void bt_run_loop(void);

// Writes up to `max` paired devices into `out`, returns how many, or -1 if
// the Bluetooth stack is unavailable (no controller, or TCC denied).
int32_t bt_list_devices(bt_device_t *out, int32_t max);

// True if the named device currently has a baseband link.
bool bt_device_connected(const char *addr);

// RFCOMM channel id advertised for `uuid16` in the device's SDP records,
// or negative on error (-1 no such service, -2 service without a channel id,
// -3 unknown device).
int32_t bt_rfcomm_channel_for_uuid(const char *addr, const uint8_t *uuid16);

// Opaque channel handle. Both transports present the same read/write API.
typedef struct bt_chan bt_chan_t;

// Open a channel. Returns NULL on failure and sets *err to the IOReturn.
bt_chan_t *bt_rfcomm_open(const char *addr, uint8_t channel_id, int32_t *err);
bt_chan_t *bt_l2cap_open(const char *addr, uint16_t psm, int32_t *err);

// Send one packet/frame. Returns 0 on success, else the IOReturn.
int32_t bt_send(bt_chan_t *chan, const uint8_t *buf, size_t len);

// Copy up to `len` buffered bytes into `buf`, waiting up to `timeout_ms` for
// some to arrive. Returns bytes copied (>0), 0 on timeout, -1 once the
// channel is closed and drained.
int32_t bt_recv(bt_chan_t *chan, uint8_t *buf, size_t len, int32_t timeout_ms);

// Largest payload the channel will carry in one send.
uint32_t bt_mtu(bt_chan_t *chan);

// Close the channel and release the handle. Safe to call once, from any thread.
void bt_close(bt_chan_t *chan);

#ifdef __cplusplus
}
#endif

#endif  // BUDS_BTSHIM_H
