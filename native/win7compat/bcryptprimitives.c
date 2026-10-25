#include <windows.h>

typedef BOOLEAN (WINAPI *RtlGenRandomFn)(void*, ULONG);
static RtlGenRandomFn s_rtl_gen_random = NULL;

__declspec(dllexport) BOOLEAN WINAPI ProcessPrng(void* pbBuffer, size_t cbBuffer) {
    if (!s_rtl_gen_random) {
        HMODULE hAdvApi = LoadLibraryA("advapi32.dll");
        if (hAdvApi) {
            s_rtl_gen_random = (RtlGenRandomFn)GetProcAddress(hAdvApi, "SystemFunction036");
        }
    }
    if (s_rtl_gen_random) {
        return s_rtl_gen_random(pbBuffer, (ULONG)cbBuffer);
    }
    return FALSE;
}

BOOL WINAPI DllMain(HINSTANCE hinstDLL, DWORD fdwReason, LPVOID lpvReserved) {
    (void)hinstDLL;
    (void)fdwReason;
    (void)lpvReserved;
    return TRUE;
}
