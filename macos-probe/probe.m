// IOBluetooth feasibility probe for the buds-tui macOS port.
//
// Answers four questions that the whole port depends on:
//   1. can we enumerate paired devices and read their state?
//   2. can we see the Maestro service record (and its RFCOMM channel id)?
//   3. can we open RFCOMM to the Pixel Buds Maestro channel?
//   4. can we open L2CAP PSM 0x1001 (AACP) to AirPods, or does macOS,
//      which holds that channel itself, refuse us?
//
// It also reports which thread the channel delegate callbacks land on,
// since the port has to decide whether the TUI can keep the main thread.
//
// build: clang -fobjc-arc -framework Foundation -framework IOBluetooth probe.m -o probe

#import <Foundation/Foundation.h>
#import <IOBluetooth/IOBluetooth.h>

static const uint16_t AACP_PSM = 0x1001;

// 25e97ff7-24ce-4c4c-8951-f764a708f7b5
static const uint8_t kMaestroUUID[16] = {
    0x25, 0xe9, 0x7f, 0xf7, 0x24, 0xce, 0x4c, 0x4c,
    0x89, 0x51, 0xf7, 0x64, 0xa7, 0x08, 0xf7, 0xb5,
};

static NSString *Thread(void) {
    return [NSThread isMainThread] ? @"main" : [NSString stringWithFormat:@"worker(%p)", [NSThread currentThread]];
}

static NSString *Hex(const void *data, size_t len) {
    NSMutableString *s = [NSMutableString string];
    const uint8_t *p = data;
    for (size_t i = 0; i < len && i < 48; i++) [s appendFormat:@"%02x ", p[i]];
    if (len > 48) [s appendString:@"..."];
    return s;
}

static void Log(NSString *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    NSString *msg = [[NSString alloc] initWithFormat:fmt arguments:ap];
    va_end(ap);
    fprintf(stdout, "%s\n", msg.UTF8String);
    fflush(stdout);
}

// Collects delegate callbacks for both channel kinds and records where they fired.
@interface Probe : NSObject <IOBluetoothRFCOMMChannelDelegate, IOBluetoothL2CAPChannelDelegate>
@property(atomic) BOOL opened;
@property(atomic) BOOL closed;
@property(atomic) IOReturn openStatus;
@property(atomic) NSUInteger bytesIn;
@property(copy) NSString *callbackThread;
@end

@implementation Probe

- (void)noteThread {
    if (!self.callbackThread) self.callbackThread = Thread();
}

- (void)rfcommChannelOpenComplete:(IOBluetoothRFCOMMChannel *)ch status:(IOReturn)error {
    [self noteThread];
    self.openStatus = error;
    self.opened = (error == kIOReturnSuccess);
    Log(@"    [cb on %@] rfcommChannelOpenComplete status=0x%08x", Thread(), error);
}

- (void)rfcommChannelData:(IOBluetoothRFCOMMChannel *)ch data:(void *)data length:(size_t)len {
    [self noteThread];
    self.bytesIn += len;
    Log(@"    [cb on %@] rfcommChannelData %zu bytes", Thread(), len);
}

- (void)rfcommChannelClosed:(IOBluetoothRFCOMMChannel *)ch {
    [self noteThread];
    self.closed = YES;
    Log(@"    [cb on %@] rfcommChannelClosed", Thread());
}

- (void)l2capChannelOpenComplete:(IOBluetoothL2CAPChannel *)ch status:(IOReturn)error {
    [self noteThread];
    self.openStatus = error;
    self.opened = (error == kIOReturnSuccess);
    static int failures = 0;
    if (error == kIOReturnSuccess) {
        Log(@"    [cb on %@] l2capChannelOpenComplete SUCCESS", Thread());
    } else if (++failures <= 3) {
        Log(@"    [cb on %@] l2capChannelOpenComplete status=0x%08x", Thread(), error);
    }
}

- (void)l2capChannelData:(IOBluetoothL2CAPChannel *)ch data:(void *)data length:(size_t)len {
    [self noteThread];
    self.bytesIn += len;
    Log(@"    <- %zu bytes: %@", len, Hex(data, len));
}

- (void)l2capChannelClosed:(IOBluetoothL2CAPChannel *)ch {
    [self noteThread];
    self.closed = YES;
    Log(@"    [cb on %@] l2capChannelClosed", Thread());
}

@end

// Spin this thread's run loop for at most `seconds`, stopping early once
// `done` goes true. IOBluetooth delivers everything through a run loop, so
// without this nothing arrives at all.
static void Pump(Probe *probe, double seconds, BOOL (^done)(void)) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([deadline timeIntervalSinceNow] > 0) {
        if (done && done()) return;
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
    }
}

static BOOL IsPixelBuds(IOBluetoothDevice *d) {
    return [d.name rangeOfString:@"Pixel Buds" options:NSCaseInsensitiveSearch].location != NSNotFound;
}

static BOOL IsAirPods(IOBluetoothDevice *d) {
    return [d.name rangeOfString:@"AirPods" options:NSCaseInsensitiveSearch].location != NSNotFound;
}

static NSString *Describe(IOBluetoothDevice *d) {
    return [NSString stringWithFormat:@"%@ [%@]%@", d.name ?: @"(unnamed)", d.addressString,
                                      d.isConnected ? @" CONNECTED" : @""];
}

// ---- 1. enumeration ------------------------------------------------------

static NSArray<IOBluetoothDevice *> *Enumerate(void) {
    Log(@"== 1. paired device enumeration ==");
    NSArray *devices = [IOBluetoothDevice pairedDevices];
    if (!devices) {
        Log(@"  pairedDevices returned nil -- no Bluetooth permission (TCC) or no controller");
        return @[];
    }
    Log(@"  %lu paired device(s)", (unsigned long)devices.count);
    for (IOBluetoothDevice *d in devices) {
        Log(@"  - %@  class=0x%06x", Describe(d), (unsigned)d.classOfDevice);
    }
    return devices;
}

// ---- 2. SDP --------------------------------------------------------------

static int MaestroChannelID(IOBluetoothDevice *d) {
    IOBluetoothSDPUUID *uuid = [IOBluetoothSDPUUID uuidWithBytes:kMaestroUUID length:16];
    IOBluetoothSDPServiceRecord *rec = [d getServiceRecordForUUID:uuid];
    if (!rec) return -1;
    BluetoothRFCOMMChannelID cid = 0;
    if ([rec getRFCOMMChannelID:&cid] != kIOReturnSuccess) return -2;
    return (int)cid;
}

static void DumpServices(NSArray<IOBluetoothDevice *> *devices) {
    Log(@"\n== 2. SDP service records ==");
    for (IOBluetoothDevice *d in devices) {
        if (!IsPixelBuds(d) && !IsAirPods(d)) continue;
        Log(@"  %@", Describe(d));
        NSArray *recs = d.services;
        if (!recs.count) {
            Log(@"    no cached SDP records (device may need to be connected once)");
        }
        for (IOBluetoothSDPServiceRecord *r in recs) {
            BluetoothRFCOMMChannelID cid = 0;
            BOOL hasRfcomm = ([r getRFCOMMChannelID:&cid] == kIOReturnSuccess);
            Log(@"    - %@%@", r.getServiceName ?: @"(unnamed service)",
                hasRfcomm ? [NSString stringWithFormat:@"  rfcomm=%d", (int)cid] : @"");
        }
        int cid = MaestroChannelID(d);
        if (cid >= 0)        Log(@"    >> Maestro service FOUND, rfcomm channel %d", cid);
        else if (cid == -2)  Log(@"    >> Maestro service found but no RFCOMM channel id");
        else                 Log(@"    >> Maestro service not present");
    }
}

// ---- 3 & 4. channel opens ------------------------------------------------

static void TryRFCOMM(IOBluetoothDevice *d) {
    Log(@"\n== 3. RFCOMM to Maestro on %@ ==", Describe(d));
    if (!d.isConnected) {
        Log(@"  SKIP: not connected. Connect the buds to this Mac and re-run.");
        return;
    }
    int cid = MaestroChannelID(d);
    if (cid < 0) {
        Log(@"  SKIP: no Maestro RFCOMM channel id from SDP (%d)", cid);
        return;
    }
    Probe *probe = [Probe new];
    IOBluetoothRFCOMMChannel *ch = nil;
    Log(@"  opening rfcomm channel %d from thread %@ ...", cid, Thread());
    IOReturn rc = [d openRFCOMMChannelSync:&ch withChannelID:(BluetoothRFCOMMChannelID)cid delegate:probe];
    Log(@"  openRFCOMMChannelSync -> 0x%08x (%@)", rc,
        rc == kIOReturnSuccess ? @"SUCCESS" : @"FAILED");
    if (rc != kIOReturnSuccess) return;
    Log(@"  mtu=%u", (unsigned)[ch getMTU]);
    Pump(probe, 3.0, ^BOOL { return probe.bytesIn > 0; });
    Log(@"  %lu bytes received unprompted; callbacks on %@",
        (unsigned long)probe.bytesIn, probe.callbackThread ?: @"(none fired)");
    [ch closeChannel];
    Pump(probe, 1.0, ^BOOL { return probe.closed; });
    Log(@"  >> RFCOMM to Pixel Buds WORKS from userspace");
}

static void SendAACPInit(IOBluetoothL2CAPChannel *ch, Probe *probe) {
    // The AirPods say nothing until spoken to. This is airpods-tui's own init
    // sequence: handshake, then feature flags, then request notifications.
    // Battery should start streaming right after the third packet.
    struct { const char *what; uint8_t bytes[16]; size_t len; } steps[] = {
        { "handshake",
          {0x00,0x00,0x04,0x00,0x01,0x00,0x02,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}, 16 },
        { "feature flags",
          {0x04,0x00,0x04,0x00,0x4d,0x00,0xff,0x00,0x00,0x00,0x00,0x00,0x00,0x00}, 14 },
        { "request notifications",
          {0x04,0x00,0x04,0x00,0x0f,0x00,0xff,0xff,0xff,0xff}, 10 },
    };
    for (size_t i = 0; i < sizeof(steps)/sizeof(steps[0]); i++) {
        Log(@"  -> %s: %@", steps[i].what, Hex(steps[i].bytes, steps[i].len));
        IOReturn w = [ch writeSync:(void *)steps[i].bytes length:(UInt16)steps[i].len];
        if (w != kIOReturnSuccess) {
            Log(@"     writeSync FAILED 0x%08x -- channel rejected our write", w);
            return;
        }
        Pump(probe, 0.6, nil);
    }
    Pump(probe, 3.0, nil);
    Log(@"  %lu bytes total from the device", (unsigned long)probe.bytesIn);
}

static void TryL2CAP(IOBluetoothDevice *d) {
    Log(@"\n== 4. L2CAP PSM 0x1001 (AACP) on %@ ==", Describe(d));
    if (!d.isConnected) {
        Log(@"  SKIP: not connected. Connect the AirPods to this Mac and re-run.");
        return;
    }
    Probe *probe = [Probe new];
    IOBluetoothL2CAPChannel *ch = nil;
    Log(@"  opening L2CAP psm 0x%04x from thread %@ ...", AACP_PSM, Thread());
    IOReturn rc = [d openL2CAPChannelSync:&ch withPSM:AACP_PSM delegate:probe];
    Log(@"  openL2CAPChannelSync -> 0x%08x (%@)", rc,
        rc == kIOReturnSuccess ? @"SUCCESS" : @"FAILED");
    if (rc != kIOReturnSuccess) {
        Log(@"  >> AACP is NOT reachable from userspace on this Mac");
        Log(@"     (expected: macOS holds the single AAP session itself)");
        return;
    }
    Log(@"  mtu in=%u out=%u", (unsigned)[ch incomingMTU], (unsigned)[ch outgoingMTU]);

    SendAACPInit(ch, probe);
    Log(@"  %lu bytes received unprompted; callbacks on %@",
        (unsigned long)probe.bytesIn, probe.callbackThread ?: @"(none fired)");
    [ch closeChannel];
    Pump(probe, 1.0, ^BOOL { return probe.closed; });
    Log(@"  >> AACP L2CAP channel OPENED from userspace");
}

// Poll until each interesting device shows up connected, testing it as soon
// as it does, so one run can cover both the buds and the AirPods.
static void RaceL2CAP(IOBluetoothDevice *d) {
    Log(@"\n== 5. racing macOS for PSM 0x1001 on %@ ==", Describe(d));
    Log(@"  dropping the baseband link (audio will cut briefly) ...");
    [d closeConnection];
    NSDate *dl = [NSDate dateWithTimeIntervalSinceNow:8];
    while (d.isConnected && [dl timeIntervalSinceNow] > 0) {
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
    }
    Log(@"  isConnected=%d after closeConnection", (int)d.isConnected);

    IOReturn oc = [d openConnection];
    Log(@"  openConnection -> 0x%08x; hammering PSM 0x1001 ...", oc);

    for (int i = 0; i < 400; i++) {
        Probe *p = [Probe new];
        IOBluetoothL2CAPChannel *ch = nil;
        IOReturn rc = [d openL2CAPChannelSync:&ch withPSM:AACP_PSM delegate:p];
        if (rc == kIOReturnSuccess) {
            Log(@"  WON the race on attempt %d (%.1fs in)", i + 1, i * 0.05);
            Log(@"  mtu in=%u out=%u", (unsigned)[ch incomingMTU], (unsigned)[ch outgoingMTU]);
            Log(@"  letting the link settle for 2s before the handshake ...");
            Pump(p, 2.0, nil);
            Log(@"  %lu bytes arrived unprompted while settling", (unsigned long)p.bytesIn);
            SendAACPInit(ch, p);
            Log(@"  listening a further 8s ...");
            Pump(p, 8.0, nil);
            Log(@"  %lu bytes total after full init", (unsigned long)p.bytesIn);
            [ch closeChannel];
            Log(@"  >> AACP IS reachable, but only by beating macOS to the channel");
            return;
        }
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
    }
    Log(@"  >> lost all 400 attempts; macOS keeps PSM 0x1001 to itself");
}

static void WaitAndTest(NSArray<IOBluetoothDevice *> *devices, double seconds) {
    NSMutableSet *tested = [NSMutableSet set];
    NSUInteger want = 0;
    for (IOBluetoothDevice *d in devices) {
        if (IsPixelBuds(d) || IsAirPods(d)) want++;
    }
    Log(@"\n== 3/4. waiting up to %.0fs for devices to connect ==", seconds);
    Log(@"  connect the Pixel Buds and the AirPods to this Mac now");

    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([deadline timeIntervalSinceNow] > 0 && tested.count < want) {
        for (IOBluetoothDevice *d in devices) {
            if (!(IsPixelBuds(d) || IsAirPods(d))) continue;
            if ([tested containsObject:d.addressString]) continue;
            if (!d.isConnected) continue;
            [tested addObject:d.addressString];
            if (IsPixelBuds(d)) TryRFCOMM(d);
            else                TryL2CAP(d);
        }
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:1.0]];
    }
    if (tested.count < want) {
        Log(@"\n  timed out with %lu of %lu device(s) tested",
            (unsigned long)tested.count, (unsigned long)want);
    }
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        double waitFor = 0;
        BOOL onWorker = NO;
        BOOL raceMode = NO;
        for (int i = 1; i < argc; i++) {
            if (strcmp(argv[i], "--worker") == 0) onWorker = YES;
            else if (strcmp(argv[i], "--wait") == 0 && i + 1 < argc) waitFor = atof(argv[++i]);
            else if (strcmp(argv[i], "--race") == 0) raceMode = YES;
        }
        Log(@"buds-tui macOS IOBluetooth probe (phases on %@ thread)\n",
            onWorker ? @"a secondary" : @"the main");

        NSArray *devices = Enumerate();
        if (!devices.count) return 1;
        DumpServices(devices);

        void (^phases)(void) = ^{
            if (raceMode) {
                for (IOBluetoothDevice *d in devices) {
                    if (IsAirPods(d) && d.isConnected) RaceL2CAP(d);
                }
                return;
            }
            if (waitFor > 0) {
                WaitAndTest(devices, waitFor);
                return;
            }
            for (IOBluetoothDevice *d in devices) {
                if (IsPixelBuds(d)) TryRFCOMM(d);
            }
            for (IOBluetoothDevice *d in devices) {
                if (IsAirPods(d)) TryL2CAP(d);
            }
        };

        if (onWorker) {
            // Does IOBluetooth work off the main thread, with that thread
            // running its own run loop? Decides whether the TUI keeps main.
            __block BOOL finished = NO;
            NSThread *t = [[NSThread alloc] initWithBlock:^{
                @autoreleasepool { phases(); }
                finished = YES;
            }];
            [t start];
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:waitFor + 60];
            while (!finished && [deadline timeIntervalSinceNow] > 0) {
                // Deliberately NOT pumping the main run loop here would be the
                // stricter test; we pump so the process stays responsive.
                [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                         beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
            }
            Log(@"\nworker thread %@", finished ? @"finished" : @"TIMED OUT (needs the main run loop)");
        } else {
            phases();
        }
        Log(@"\ndone.");
    }
    return 0;
}
