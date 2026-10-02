// C ABI over macOS IOBluetooth. See btshim.h for the contract.
//
// Everything IOBluetooth does arrives through a run loop, so this owns one
// thread that does nothing but run one, and hops every call onto it. Inbound
// bytes are buffered behind a condition variable, which is what lets the Rust
// side read with a plain blocking call instead of trying to bridge delegates.

#import <Foundation/Foundation.h>
#import <IOBluetooth/IOBluetooth.h>
#import "btshim.h"

// ---- the run loop thread -------------------------------------------------

@interface BTShim : NSObject
@property(strong) NSThread *thread;
@property(atomic) BOOL ready;
@end

@implementation BTShim

+ (instancetype)shared {
    static BTShim *shim;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        shim = [BTShim new];
        [shim start];
    });
    return shim;
}

- (void)start { self.ready = YES; }

- (void)invoke:(void (^)(void))block { block(); }

// Runs `block` on the main thread and waits for it. Opening an RFCOMM or
// L2CAP channel from any other thread fails with kIOReturnError even when
// that thread runs its own run loop, so main is not negotiable here.
- (void)sync:(void (^)(void))block {
    if ([NSThread isMainThread]) { block(); return; }
    [self performSelectorOnMainThread:@selector(invoke:)
                           withObject:[block copy]
                        waitUntilDone:YES];
}

@end

static void Dispatch(void (^block)(void)) { [[BTShim shared] sync:block]; }

// ---- channels ------------------------------------------------------------

// One of these backs every bt_chan_t. Both transports land in the same inbox,
// so bt_recv does not care which kind it is reading from.
@interface BTChannel : NSObject <IOBluetoothRFCOMMChannelDelegate, IOBluetoothL2CAPChannelDelegate>
@property(strong) IOBluetoothRFCOMMChannel *rfcomm;
@property(strong) IOBluetoothL2CAPChannel *l2cap;
@property(strong) NSCondition *cond;
// A queue of frames rather than one flat buffer: L2CAP is datagram-oriented
// and AACP packet boundaries are meaningful, so flattening would lose them.
@property(strong) NSMutableArray<NSData *> *inbox;
@property(assign) NSUInteger headOffset;
@property(atomic) BOOL closed;
@property(atomic) BOOL openFailed;
@end

@implementation BTChannel

- (instancetype)init {
    if ((self = [super init])) {
        _cond = [NSCondition new];
        _inbox = [NSMutableArray array];
    }
    return self;
}

- (void)deliver:(const void *)bytes length:(size_t)len {
    NSData *frame = [NSData dataWithBytes:bytes length:len];
    [self.cond lock];
    [self.inbox addObject:frame];
    [self.cond broadcast];
    [self.cond unlock];
}

- (void)markClosed {
    [self.cond lock];
    self.closed = YES;
    [self.cond broadcast];
    [self.cond unlock];
}

- (void)rfcommChannelData:(IOBluetoothRFCOMMChannel *)c data:(void *)d length:(size_t)l {
    [self deliver:d length:l];
}
- (void)rfcommChannelClosed:(IOBluetoothRFCOMMChannel *)c { [self markClosed]; }
- (void)l2capChannelData:(IOBluetoothL2CAPChannel *)c data:(void *)d length:(size_t)l {
    [self deliver:d length:l];
}
- (void)l2capChannelClosed:(IOBluetoothL2CAPChannel *)c { [self markClosed]; }

@end

// ---- devices -------------------------------------------------------------

static IOBluetoothDevice *DeviceAt(const char *addr) {
    if (!addr) return nil;
    return [IOBluetoothDevice deviceWithAddressString:[NSString stringWithUTF8String:addr]];
}

void bt_init(void) { (void)[BTShim shared]; }

void bt_run_loop(void) {
    // Hands the main thread to a run loop forever. IOBluetooth delivers its
    // delegate callbacks here, so the host must call this on the main thread
    // and run everything else (the TUI, the async runtime) elsewhere.
    @autoreleasepool {
        [[NSRunLoop mainRunLoop] addPort:[NSMachPort port] forMode:NSDefaultRunLoopMode];
    }
    while (1) {
        @autoreleasepool {
            [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode
                                  beforeDate:[NSDate distantFuture]];
        }
    }
}

int32_t bt_list_devices(bt_device_t *out, int32_t max) {
    if (!out || max <= 0) return 0;
    __block int32_t n = 0;
    Dispatch(^{
        NSArray *devices = [IOBluetoothDevice pairedDevices];
        if (!devices) { n = -1; return; }
        for (IOBluetoothDevice *d in devices) {
            if (n >= max) break;
            bt_device_t *slot = &out[n];
            memset(slot, 0, sizeof(*slot));
            strncpy(slot->name, (d.name ?: @"").UTF8String, BT_NAME_LEN - 1);
            strncpy(slot->addr, (d.addressString ?: @"").UTF8String, BT_ADDR_LEN - 1);
            slot->connected = d.isConnected;
            n++;
        }
    });
    return n;
}

bool bt_device_connected(const char *addr) {
    __block bool connected = false;
    Dispatch(^{ connected = DeviceAt(addr).isConnected; });
    return connected;
}

int32_t bt_rfcomm_channel_for_uuid(const char *addr, const uint8_t *uuid16) {
    if (!uuid16) return -1;
    __block int32_t result = -3;
    Dispatch(^{
        IOBluetoothDevice *d = DeviceAt(addr);
        if (!d) return;
        IOBluetoothSDPUUID *uuid = [IOBluetoothSDPUUID uuidWithBytes:uuid16 length:16];
        IOBluetoothSDPServiceRecord *rec = [d getServiceRecordForUUID:uuid];
        if (!rec) { result = -1; return; }
        BluetoothRFCOMMChannelID cid = 0;
        result = ([rec getRFCOMMChannelID:&cid] == kIOReturnSuccess) ? (int32_t)cid : -2;
    });
    return result;
}

// ---- opening -------------------------------------------------------------

// Handles are the BTChannel itself, retained until bt_close.
struct bt_chan { void *obj; };

static bt_chan_t *WrapOpened(BTChannel *ch) {
    bt_chan_t *handle = calloc(1, sizeof(bt_chan_t));
    handle->obj = (void *)CFBridgingRetain(ch);
    return handle;
}

static BTChannel *Unwrap(bt_chan_t *handle) {
    return handle ? (__bridge BTChannel *)handle->obj : nil;
}

bt_chan_t *bt_rfcomm_open(const char *addr, uint8_t channel_id, int32_t *err) {
    __block IOReturn rc = kIOReturnError;
    BTChannel *ch = [BTChannel new];
    Dispatch(^{
        IOBluetoothDevice *d = DeviceAt(addr);
        if (!d) { rc = kIOReturnNoDevice; return; }
        IOBluetoothRFCOMMChannel *opened = nil;
        rc = [d openRFCOMMChannelSync:&opened
                        withChannelID:(BluetoothRFCOMMChannelID)channel_id
                             delegate:ch];
        if (rc == kIOReturnSuccess) ch.rfcomm = opened;
    });
    if (err) *err = (int32_t)rc;
    return (rc == kIOReturnSuccess) ? WrapOpened(ch) : NULL;
}

bt_chan_t *bt_l2cap_open(const char *addr, uint16_t psm, int32_t *err) {
    __block IOReturn rc = kIOReturnError;
    BTChannel *ch = [BTChannel new];
    Dispatch(^{
        IOBluetoothDevice *d = DeviceAt(addr);
        if (!d) { rc = kIOReturnNoDevice; return; }
        IOBluetoothL2CAPChannel *opened = nil;
        rc = [d openL2CAPChannelSync:&opened withPSM:(BluetoothL2CAPPSM)psm delegate:ch];
        if (rc == kIOReturnSuccess) ch.l2cap = opened;
    });
    if (err) *err = (int32_t)rc;
    return (rc == kIOReturnSuccess) ? WrapOpened(ch) : NULL;
}

// ---- io ------------------------------------------------------------------

int32_t bt_send(bt_chan_t *handle, const uint8_t *buf, size_t len) {
    BTChannel *ch = Unwrap(handle);
    if (!ch || ch.closed) return kIOReturnNotOpen;
    __block IOReturn rc = kIOReturnError;
    Dispatch(^{
        if (ch.rfcomm) {
            rc = [ch.rfcomm writeSync:(void *)buf length:(UInt16)len];
        } else if (ch.l2cap) {
            rc = [ch.l2cap writeSync:(void *)buf length:(UInt16)len];
        } else {
            rc = kIOReturnNotOpen;
        }
    });
    return (int32_t)rc;
}

int32_t bt_recv(bt_chan_t *handle, uint8_t *buf, size_t len, int32_t timeout_ms) {
    BTChannel *ch = Unwrap(handle);
    if (!ch || !buf || len == 0) return -1;

    [ch.cond lock];
    if (ch.inbox.count == 0 && !ch.closed) {
        [ch.cond waitUntilDate:[NSDate dateWithTimeIntervalSinceNow:timeout_ms / 1000.0]];
    }
    if (ch.inbox.count == 0) {
        BOOL closed = ch.closed;
        [ch.cond unlock];
        return closed ? -1 : 0;   // drained and closed, vs. just nothing yet
    }
    // One frame at a time, so a caller with a buffer at least as big as the
    // MTU always sees exactly the packets the device sent.
    NSData *head = ch.inbox[0];
    NSUInteger remaining = head.length - ch.headOffset;
    NSUInteger n = MIN(remaining, len);
    memcpy(buf, (const uint8_t *)head.bytes + ch.headOffset, n);
    if (n == remaining) {
        [ch.inbox removeObjectAtIndex:0];
        ch.headOffset = 0;
    } else {
        ch.headOffset += n;
    }
    [ch.cond unlock];
    return (int32_t)n;
}

uint32_t bt_mtu(bt_chan_t *handle) {
    BTChannel *ch = Unwrap(handle);
    if (!ch) return 0;
    __block uint32_t mtu = 0;
    Dispatch(^{
        if (ch.rfcomm)      mtu = [ch.rfcomm getMTU];
        else if (ch.l2cap)  mtu = [ch.l2cap outgoingMTU];
    });
    return mtu;
}

void bt_close(bt_chan_t *handle) {
    BTChannel *ch = Unwrap(handle);
    if (!ch) return;
    Dispatch(^{
        [ch.rfcomm closeChannel];
        [ch.l2cap closeChannel];
    });
    [ch markClosed];
    CFBridgingRelease(handle->obj);
    handle->obj = NULL;
    free(handle);
}
