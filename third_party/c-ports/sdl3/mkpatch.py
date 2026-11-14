"""Vyrobi zaplatu 0001-video-magnumopus.patch pro SDL 3.4.20.

Pouziti (adresare mimo repozitar, napr. target/c-ports/study):
  tar -xf SDL3-3.4.20.tar.gz -C pa && tar -xf SDL3-3.4.20.tar.gz -C pb
  python -I mkpatch.py pb/SDL3-3.4.20
  git diff --no-index --no-prefix pa/SDL3-3.4.20 pb/SDL3-3.4.20 > zaplata
  (a v hlavickach diffu prepsat cesty na a/ a b/, -p1)
Zaplata se pak pouziva v xtask/src/cports (docs/cs/porty-c.md).
"""
import sys

root = sys.argv[1]


def rep(p, a, b):
    path = root + '/' + p
    s = open(path, encoding='utf-8', newline='').read()
    assert a in s, (p, a)
    s = s.replace(a, b, 1)
    open(path, 'w', encoding='utf-8', newline='').write(s)


rep('CMakeLists.txt', '''# Platform-independent options

if(SDL_VIDEO)
''', '''# Platform-independent options

# MagnumOpus: okno kompozitoru pres libs/mosdl (modul platformy z xtasku
# nastavi MAGNUMOPUS).
if(SDL_VIDEO AND MAGNUMOPUS)
  set(SDL_VIDEO_DRIVER_MAGNUMOPUS 1)
  sdl_glob_sources(
    "${SDL3_SOURCE_DIR}/src/video/magnumopus/*.c"
  )
  set(HAVE_SDL_VIDEO TRUE)
endif()

if(SDL_VIDEO)
''')
# SDL_OpenURL na unixu spousti xdg-open pres fork: na MagnumOpus zatim
# misc/dummy (pozdeji sluzba "otevri cestu").
rep('CMakeLists.txt', '''  sdl_glob_sources("${SDL3_SOURCE_DIR}/src/misc/unix/*.c")
  set(HAVE_SDL_MISC TRUE)
''', '''  if(NOT MAGNUMOPUS)
    sdl_glob_sources("${SDL3_SOURCE_DIR}/src/misc/unix/*.c")
    set(HAVE_SDL_MISC TRUE)
  endif()
''')
rep('include/build_config/SDL_build_config.h.cmake', '#cmakedefine SDL_VIDEO_DRIVER_KMSDRM 1\n',
    '#cmakedefine SDL_VIDEO_DRIVER_KMSDRM 1\n#cmakedefine SDL_VIDEO_DRIVER_MAGNUMOPUS 1\n')
rep('src/video/SDL_video.c', '#ifdef SDL_VIDEO_DRIVER_OFFSCREEN\n    &OFFSCREEN_bootstrap,',
    '#ifdef SDL_VIDEO_DRIVER_MAGNUMOPUS\n    &MAGNUMOPUS_bootstrap,\n#endif\n#ifdef SDL_VIDEO_DRIVER_OFFSCREEN\n    &OFFSCREEN_bootstrap,')
rep('src/video/SDL_sysvideo.h', 'extern VideoBootStrap OFFSCREEN_bootstrap;',
    'extern VideoBootStrap MAGNUMOPUS_bootstrap;\nextern VideoBootStrap OFFSCREEN_bootstrap;')
rep('include/SDL3/SDL_platform_defines.h', '#ifdef __HAIKU__\n', '''#ifdef __magnumopus__

/**
 * A preprocessor macro that is only defined if compiling for MagnumOpus.
 *
 * \\since This macro is available since SDL 3.4.20 (MagnumOpus port).
 */
#define SDL_PLATFORM_MAGNUMOPUS 1
/* System POSIX (cstd): unixove cesty SDL (vlakna pthread, cas, soubory).
   Jen makro SDL, ne __unix__ (to by se dostalo i do aplikaci). */
#define SDL_PLATFORM_UNIX 1
#endif

#ifdef __HAIKU__
''')
# GTK (lista, dialogy) tu neni a dlopen ve statickem programu take ne:
# jen prazdne funkce, ktere vola SDL.c.
rep('src/core/unix/SDL_gtk.c', '''#include "SDL_internal.h"
#include "SDL_gtk.h"

#include <dlfcn.h>''', '''#include "SDL_internal.h"
#include "SDL_gtk.h"

#ifdef SDL_PLATFORM_MAGNUMOPUS
/* MagnumOpus: bez GTK (lista a dialogy maji vlastni rozhrani systemu). */
bool SDL_CanUseGtk(void) { return false; }
bool SDL_Gtk_Init(void) { return false; }
void SDL_Gtk_Quit(void) {}
SDL_GtkContext *SDL_Gtk_EnterContext(void) { return NULL; }
void SDL_Gtk_ExitContext(SDL_GtkContext *ctx) { (void)ctx; }
void SDL_UpdateGtk(void) {}
#else
#include <dlfcn.h>''')
path = root + '/src/core/unix/SDL_gtk.c'
s = open(path, encoding='utf-8', newline='').read()
nl = '\r\n' if '\r\n' in s else '\n'
s = s.rstrip() + nl + '#endif /* SDL_PLATFORM_MAGNUMOPUS */' + nl
open(path, 'w', encoding='utf-8', newline='').write(s)
# Tabulka kodu klaves Linuxu (evdev): ovladac magnumopus dostava kody evdev.
rep('src/events/SDL_scancode_tables.c',
    '#if defined(SDL_INPUT_LINUXEV) || defined(SDL_VIDEO_DRIVER_WAYLAND) || defined(SDL_VIDEO_DRIVER_X11)',
    '#if defined(SDL_INPUT_LINUXEV) || defined(SDL_VIDEO_DRIVER_WAYLAND) || defined(SDL_VIDEO_DRIVER_X11) || defined(SDL_VIDEO_DRIVER_MAGNUMOPUS)')
# Nas test portu (overlay test/testmagnumopus.c), jen pro MagnumOpus.
rep('test/CMakeLists.txt',
    'add_sdl_test_executable(checkkeys SOURCES checkkeys.c)\n',
    'add_sdl_test_executable(checkkeys SOURCES checkkeys.c)\nif(MAGNUMOPUS)\n  add_sdl_test_executable(testmagnumopus SOURCES testmagnumopus.c)\nendif()\n')
