/*
 * libcplug.so - plugin v C pro test libc.so (cdyn): pouziva funkce libc.so
 * (snprintf, malloc) a vlastni TLS, ktere po dlopen vznika pro kazde
 * vlakno dynamicky (DTV, DL-4b).
 */
#include <stdio.h>
#include <stdlib.h>

static _Thread_local int counter;

int plug_tick(void) { return ++counter; }

char *plug_hello(const char *who) {
    char *s = malloc(64);
    if (s != NULL) snprintf(s, 64, "ahoj %s z pluginu", who);
    return s;
}
