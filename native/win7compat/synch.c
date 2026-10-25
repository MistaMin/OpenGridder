#include <windows.h>

__declspec(dllexport) BOOL WINAPI WaitOnAddress(
    volatile VOID *Address,
    PVOID CompareAddress,
    SIZE_T AddressSize,
    DWORD dwMilliseconds
) {
    DWORD start = GetTickCount();
    while (1) {
        if (AddressSize == 1) {
            if (*(volatile char*)Address != *(char*)CompareAddress) return TRUE;
        } else if (AddressSize == 2) {
            if (*(volatile short*)Address != *(short*)CompareAddress) return TRUE;
        } else if (AddressSize == 4) {
            if (*(volatile long*)Address != *(long*)CompareAddress) return TRUE;
        } else if (AddressSize == 8) {
            if (*(volatile long long*)Address != *(long long*)CompareAddress) return TRUE;
        }
        if (dwMilliseconds != INFINITE) {
            DWORD elapsed = GetTickCount() - start;
            if (elapsed >= dwMilliseconds) {
                SetLastError(ERROR_TIMEOUT);
                return FALSE;
            }
        }
        Sleep(1);
    }
    return TRUE;
}

__declspec(dllexport) VOID WINAPI WakeByAddressSingle(PVOID Address) {
    (void)Address;
}

__declspec(dllexport) VOID WINAPI WakeByAddressAll(PVOID Address) {
    (void)Address;
}

BOOL WINAPI DllMain(HINSTANCE hinstDLL, DWORD fdwReason, LPVOID lpvReserved) {
    (void)hinstDLL;
    (void)fdwReason;
    (void)lpvReserved;
    return TRUE;
}
