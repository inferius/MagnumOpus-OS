// Knihovna pro dlopen s 4 KiB TLS initial-exec (DL-4b, libdlbig.so.1):
// do vychozi rezervy (2 KiB) se nevejde, s rezervou 8 KiB z manifestu ano.

__thread char big_tls[4096] = {5};

int big_get(int i) {
    return big_tls[i];
}
