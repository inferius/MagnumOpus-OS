/* magnumopus/sys.h - objekty jadra pro programy v C (libs/mosys).
 *
 * Kanaly, VMO, ploty, udalosti a cekani na vic objektu nad syscally
 * MagnumOpus; pro sluzby a pomocne procesy v C (proces dekoderu medii
 * ffdec), ne pro bezne aplikace (ty maji POSIX z cstd). Funkce vraci 0
 * nebo zaporny kod chyby jadra (docs/cs/chyby.md), napr. MO_ERR_SHOULD_WAIT.
 * Handle je cislo procesu; 0 = zadny. */
#ifndef _MAGNUMOPUS_SYS_H
#define _MAGNUMOPUS_SYS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint32_t mo_handle_t;

/* Kody chyb (stejne cislo jako v jadre). */
#define MO_ERR_TIMED_OUT (-4)
#define MO_ERR_NO_MEMORY (-5)
#define MO_ERR_INVALID_ARGS (-7)
#define MO_ERR_SHOULD_WAIT (-12)
#define MO_ERR_PEER_CLOSED (-13)
#define MO_ERR_BUFFER_TOO_SMALL (-14)

/* Signaly objektu (mo_wait_many). */
#define MO_SIG_SIGNALED 0x1u
#define MO_SIG_TERMINATED 0x2u
#define MO_SIG_READABLE 0x4u
#define MO_SIG_PEER_CLOSED 0x8u

/* Termin cekani: navzdy / jen zkusit. Jinak absolutni monotonni cas v ns. */
#define MO_DEADLINE_INFINITE (-1)
#define MO_DEADLINE_NOW 0

/* Monotonni cas od startu systemu (ns). */
int64_t mo_now_ns(void);

/* Bootstrap kanal od rodice (patri behovemu prostredi, nezavirat); 0 = neni. */
mo_handle_t mo_bootstrap(void);

/* Netagovane handly startovni zpravy (jen jednou; volajici je vlastni).
 * Vrati pocet zapsanych do out (nejvic max; dalsi se zavrou). */
size_t mo_take_handles(mo_handle_t *out, size_t max);

int mo_handle_close(mo_handle_t h);
int mo_handle_dup(mo_handle_t h, mo_handle_t *out);

/* Zprava do kanalu; handly se predaji vzdy (i pri chybe). */
int mo_channel_write(mo_handle_t ch, const void *data, size_t len, const mo_handle_t *handles, size_t nhandles);

/* Precte zpravu (ceka do terminu). Kratky buffer = MO_ERR_BUFFER_TOO_SMALL
 * a v *len / *nhandles potrebna velikost (zprava zustava ve fronte). */
int mo_channel_read(mo_handle_t ch, void *buf, size_t cap, size_t *len, mo_handle_t *handles, size_t hcap, size_t *nhandles, int64_t deadline_ns);

/* Namapuje len bajtu VMO od offsetu (zaokrouhli na stranku); write = i zapis. */
int mo_vmo_map(mo_handle_t vmo, uint64_t offset, uint64_t len, int write, void **addr);
int mo_vmo_unmap(void *addr, uint64_t len);

/* Plot: posunout na hodnotu / precist hodnotu. */
int mo_fence_signal(mo_handle_t fence, uint64_t value);
int mo_fence_value(mo_handle_t fence, uint64_t *value);

/* Udalost: zrusit signal SIGNALED. */
int mo_event_reset(mo_handle_t ev);

/* Ceka, az ma aspon jeden objekt nektery signal z masky; observed[i] =
 * signaly objektu i. Vrati index prvniho splneneho (>= 0) nebo chybu. */
int mo_wait_many(const mo_handle_t *handles, const uint32_t *masks, uint32_t *observed, size_t n, int64_t deadline_ns);

#ifdef __cplusplus
}
#endif

#endif
