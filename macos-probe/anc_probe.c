// Probes the undocumented CoreAudio listening-mode properties that macOS
// exposes on AirPods audio devices.
//
// This is the route AACP cannot give us: rather than speaking Apple's control
// protocol ourselves, we ask macOS's audio stack — which already owns the AACP
// session — to carry the change for us.
//
//   lstm  current listening mode   1 off, 2 noise cancellation,
//                                  3 transparency, 4 adaptive
//   lsms  bitmask of supported modes
//
// build: clang -framework CoreAudio -framework CoreFoundation anc_probe.c -o anc_probe
// usage: ./anc_probe            list devices and read the properties
//        ./anc_probe --set N    write mode N to the first device that has lstm

#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LSTM 'lstm'
#define LSMS 'lsms'

static const char *ModeName(UInt32 m) {
    switch (m) {
        case 1: return "Off";
        case 2: return "Noise Cancellation";
        case 3: return "Transparency";
        case 4: return "Adaptive";
        default: return "unknown";
    }
}

static char *DeviceName(AudioObjectID dev) {
    AudioObjectPropertyAddress addr = {
        kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    CFStringRef name = NULL;
    UInt32 size = sizeof name;
    if (AudioObjectGetPropertyData(dev, &addr, 0, NULL, &size, &name) != noErr || !name) return NULL;
    CFIndex max = CFStringGetMaximumSizeForEncoding(CFStringGetLength(name), kCFStringEncodingUTF8) + 1;
    char *buf = calloc(1, max);
    CFStringGetCString(name, buf, max, kCFStringEncodingUTF8);
    CFRelease(name);
    return buf;
}

static int ReadU32(AudioObjectID dev, AudioObjectPropertySelector sel, UInt32 *out) {
    AudioObjectPropertyAddress addr = {
        sel, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    if (!AudioObjectHasProperty(dev, &addr)) return 0;
    UInt32 size = sizeof *out;
    return AudioObjectGetPropertyData(dev, &addr, 0, NULL, &size, out) == noErr;
}

static OSStatus WriteU32(AudioObjectID dev, AudioObjectPropertySelector sel, UInt32 value) {
    AudioObjectPropertyAddress addr = {
        sel, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    Boolean settable = false;
    OSStatus rc = AudioObjectIsPropertySettable(dev, &addr, &settable);
    if (rc != noErr) return rc;
    if (!settable) {
        fprintf(stderr, "  lstm is not settable on this device\n");
        return kAudioHardwareUnsupportedOperationError;
    }
    return AudioObjectSetPropertyData(dev, &addr, 0, NULL, sizeof value, &value);
}

int main(int argc, char **argv) {
    long want = -1;
    if (argc == 3 && strcmp(argv[1], "--set") == 0) want = strtol(argv[2], NULL, 10);

    AudioObjectPropertyAddress devs = {
        kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &devs, 0, NULL, &size) != noErr) {
        fprintf(stderr, "could not size the device list\n");
        return 1;
    }
    UInt32 count = size / sizeof(AudioObjectID);
    AudioObjectID *ids = calloc(count, sizeof(AudioObjectID));
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &devs, 0, NULL, &size, ids) != noErr) {
        fprintf(stderr, "could not read the device list\n");
        return 1;
    }

    printf("%u audio device(s)\n", count);
    int found = 0;
    for (UInt32 i = 0; i < count; i++) {
        char *name = DeviceName(ids[i]);
        UInt32 mode = 0, supported = 0;
        int has_mode = ReadU32(ids[i], LSTM, &mode);
        int has_supported = ReadU32(ids[i], LSMS, &supported);
        if (!has_mode && !has_supported) {
            printf("  - %-34s (no listening-mode properties)\n", name ? name : "?");
            free(name);
            continue;
        }
        found = 1;
        printf("  * %-34s LISTENING MODE PROPERTIES PRESENT\n", name ? name : "?");
        if (has_mode) printf("      lstm = %u (%s)\n", mode, ModeName(mode));
        if (has_supported) {
            printf("      lsms = 0x%02x ->", supported);
            for (UInt32 m = 1; m <= 4; m++) {
                if (supported & (1u << (m - 1))) printf(" %s", ModeName(m));
            }
            printf("\n");
        }
        if (want >= 0) {
            printf("      writing lstm = %ld (%s) ...\n", want, ModeName((UInt32)want));
            OSStatus rc = WriteU32(ids[i], LSTM, (UInt32)want);
            printf("      AudioObjectSetPropertyData -> %d (%s)\n", (int)rc,
                   rc == noErr ? "OK" : "failed");
            if (rc == noErr && ReadU32(ids[i], LSTM, &mode)) {
                printf("      lstm now = %u (%s)\n", mode, ModeName(mode));
            }
            want = -1;  // first capable device only
        }
        free(name);
    }
    if (!found) printf("\nNo device exposed lstm/lsms. Are the AirPods connected AND the active output?\n");
    return found ? 0 : 2;
}
