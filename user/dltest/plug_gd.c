// Druha cast libdlplug.so.1: pristup k TLS modelem general-dynamic
// (x86_64 __tls_get_addr; aarch64 i tady TLSDESC).

extern __thread int plug_tls;

int plug_tls_gd(void) {
    return plug_tls;
}
