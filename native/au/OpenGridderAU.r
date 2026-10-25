#define TARGET_OS_MAC 1
#define thng_RezTemplateVersion 1
#include "MacTypes.r"
#include "Components.r"

resource 'STR ' (1000) { "OpenGrid Studio: OpenGridder" };
resource 'STR ' (1001) { "OpenGridder Legacy AU Plugin" };

resource 'thng' (1000, "OpenGridder") {
    'aufx',
    'OGFx',
    'OpGr',
    0,
    0,
    0, 0,
    'STR ', 1000,
    'STR ', 1001,
    0, 0,
    0x00010201,
    componentHasMultiplePlatforms | componentDoAutoVersion,
    0,
    {
        0, 'dlle', 1000, platformIA32NativeEntryPoint,
        0, 'dlle', 1000, platformX86_64NativeEntryPoint
    }
};

resource 'dlle' (1000) {
    "OpenGridderAUEntry"
};
