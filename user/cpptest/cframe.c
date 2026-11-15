/*
 * Ramec v C mezi dvema ramci C++ (test cpptest): vyjimka C++ vyhozena ve
 * zpetnem volani musi projit timto ramcem (tabulky rozbalovani z
 * -funwind-tables v sysrootu C, docs/cs/cpp.md).
 */

static volatile int after_callback;

int cframe_call(void (*cb)(void *), void *arg)
{
    after_callback = 0;
    cb(arg);
    /* Sem se pri vyjimce nedojde. */
    after_callback = 1;
    return 7;
}

int cframe_after(void)
{
    return after_callback;
}
