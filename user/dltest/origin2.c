// Knihovna z disku (DL-4c, libdlorigin2.so.1): zavisi na systemove
// libdltest2 (/system/lib od libd).

extern int dl2_add(int);

int orig2_fn(int x) {
    return dl2_add(x) * 2;
}
