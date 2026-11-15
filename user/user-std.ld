/*
 * Linker skript aplikaci se std (cil *-unknown-magnumopus, x86_64 i aarch64).
 *
 * Stejne rozlozeni jako user/user.ld (staticky ET_EXEC od 4 MiB, skupiny
 * sekci R-X, R--, RW- na vlastnich strankach zarovnanych na 64 KiB) a navic
 * TLS: .tdata i .tbss jsou v jedne vystupni sekci, takze cela sablona TLS
 * (vcetne nul z .tbss) je v souboru a runtime (libs/osrt, tls.rs) ji jen
 * zkopiruje do bloku vlakna. Segment PT_TLS urcuje posuny promennych
 * (p_memsz, p_align); symboly __tls_* je predavaji runtime.
 *
 * Rozbalovani (C++, libunwind): hlavicky ELF v pameti nejsou, proto
 * staticky dl_iterate_phdr (libs/cstd/src/dlfcn.rs) sklada PT_LOAD
 * a PT_GNU_EH_FRAME ze symbolu __mos_image_start/end a
 * __eh_frame_hdr_start/end. Obsah .eh_frame_hdr vytvori lld
 * s --eh-frame-hdr (rustc ho predava sam, C pres clang.cfg).
 */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R-X */
    rodata PT_LOAD FLAGS(4);   /* R-- */
    data   PT_LOAD FLAGS(6);   /* RW- */
    tls    PT_TLS;
}

SECTIONS
{
    . = 0x400000;
    __mos_image_start = .;

    .text : ALIGN(64K) { *(.text .text.*) } :text
    .rodata : ALIGN(64K) { *(.rodata .rodata.*) } :rodata
    .eh_frame_hdr : {
        __eh_frame_hdr_start = .;
        *(.eh_frame_hdr)
        __eh_frame_hdr_end = .;
    } :rodata
    .eh_frame : { KEEP(*(.eh_frame)) } :rodata
    .gcc_except_table : { *(.gcc_except_table .gcc_except_table.*) } :rodata
    .data : ALIGN(64K) { *(.data .data.*) *(.got .got.*) } :data
    /* Konstruktory a destruktory (C++ staticke objekty, __attribute__
     * constructor); spousti je osrt (start.rs) pred main a pri exit. */
    .init_array : {
        __init_array_start = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*) .init_array))
        __init_array_end = .;
    } :data
    .fini_array : {
        __fini_array_start = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.fini_array.*) .fini_array))
        __fini_array_end = .;
    } :data
    .tdata : {
        __tls_template_start = .;
        *(.tdata .tdata.*)
        *(.tbss .tbss.*)
        *(.tcommon)
        /* Vynuti PROGBITS: bez .tdata by sekce byla NOBITS a lld by jeji
         * adresy prekryl s nasledujicimi sekcemi (sablona by nebyla nuly). */
        BYTE(0)
    } :data :tls
    __tls_template_size = SIZEOF(.tdata);
    __tls_align = ALIGNOF(.tdata);
    .bss : ALIGN(4K) { *(.bss .bss.*) *(COMMON) } :data
    __mos_image_end = .;

    /DISCARD/ : { *(.comment) *(.note .note.*) }
}
