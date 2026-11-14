// Knihovna pro dlopen s TLS initial-exec (DL-4b, libdlstatic.so.1,
// DF_STATIC_TLS): zavadec ji da do rezervy statickeho TLS, stejny posun od
// TP ve vsech vlaknech.

__thread int st_tls = 33;

int st_bump(void) {
    return ++st_tls;
}
