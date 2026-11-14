/*
 * cdyn-hello - program PIE prelozeny jen pres `clang --config=clang-dyn.cfg`
 * ze sysrootu C (cesta pro cizi build systemy), bez cbuild.
 */
#include <stdio.h>

int main(int argc, char **argv) {
    printf("ok hello %d %s\ncdyn-hello: hotovo\n", argc, argv[0]);
    return 0;
}
